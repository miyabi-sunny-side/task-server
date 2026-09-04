//! One-way import of the markdown task queue that came before sqlite.
//!
//! The queue was a directory of markdown files with YAML frontmatter: a live
//! queue and an archive. This reads them and writes rows, all or nothing, and
//! never touches the directory it read. Deleting the markdown afterwards is a
//! decision for whoever ran the import, not for the import.
//!
//! Two rules keep the migration honest. Every v0.1 status is named in
//! [`RENAMED`] or kept by the v0.2 vocabulary itself, so an unrecognised one
//! stops the import instead of being quietly dropped. And every frontmatter key
//! the schema has no column for is folded into the body as a marked YAML block,
//! including the pre-mapping status, so nothing that was written down is lost
//! and the mapping can be read backwards.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_norway::{Mapping, Value};
use time::OffsetDateTime;

use crate::clock::{Clock, format_z};
use crate::db::Db;
use crate::error::Error;
use crate::frontmatter::{get_str, serialize_mapping, split_document};
use crate::product::{self, check_product_id};
use crate::task::{self, ALL_STATUSES, TaskKind, TaskStatus};

/// The v0.1 statuses that did not survive under their own name.
///
/// `done` is the one that matters: in v0.1 it meant "the human accepted it and
/// the work is over", which is `merged` here. Mapping it to `done` would raise
/// every finished task of the old queue as a merge candidate.
const RENAMED: [(&str, TaskStatus); 5] = [
    ("running", TaskStatus::Wip),
    ("awaiting_user", TaskStatus::Done),
    ("done", TaskStatus::Merged),
    ("release_requested", TaskStatus::Merged),
    ("release_failed", TaskStatus::Merged),
];

/// Frontmatter keys that always reach a column of their own, and so are not
/// repeated in the folded block. The original `status` is the one exception: it
/// is always folded in, because the mapping is not reversible without it.
///
/// The product keys are deliberately absent: whether `target_space` or
/// `product_id` reaches its column depends on the value, and a key whose value
/// stayed behind has to stay in the block with it.
const MAPPED_KEYS: [&str; 5] = [
    "title",
    "status",
    "commit_sha",
    "verification",
    "release_tag",
];

const BLOCK_HEADING: &str = "---\n\n## Imported v0.1 metadata\n\n```yaml\n";

/// Where the markdown is. Either directory alone is a valid import; both
/// omitted is not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSources {
    pub live: Option<PathBuf>,
    pub archive: Option<PathBuf>,
}

impl ImportSources {
    /// Parse `--live <DIR>` and `--archive <DIR>` out of the arguments that
    /// follow the subcommand.
    pub fn from_args(args: &[String]) -> Result<Self, Error> {
        let mut sources = Self::default();
        let mut rest = args.iter();
        while let Some(flag) = rest.next() {
            let slot = match flag.as_str() {
                "--live" => &mut sources.live,
                "--archive" => &mut sources.archive,
                other => {
                    return Err(Error::Invalid(format!(
                        "unknown argument '{other}'; expected --live <DIR> and/or --archive <DIR>"
                    )));
                }
            };
            let value = rest
                .next()
                .ok_or_else(|| Error::Invalid(format!("{flag} needs a directory")))?;
            *slot = Some(PathBuf::from(value));
        }
        if sources.live.is_none() && sources.archive.is_none() {
            return Err(Error::Invalid(
                "at least one of --live <DIR> or --archive <DIR> is required".into(),
            ));
        }
        Ok(sources)
    }
}

/// One file the import refused, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub path: PathBuf,
    pub reason: String,
}

#[derive(Debug)]
pub enum ImportError {
    /// Files the import refused. Nothing was written; every reason is here, so
    /// one run tells the operator about every file that needs attention.
    Refused(Vec<Problem>),
    /// The import could not run at all: no source, or the database refused.
    Failed(Error),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(problems) => {
                writeln!(
                    f,
                    "import refused ({} problem(s)); nothing was written",
                    problems.len()
                )?;
                for problem in problems {
                    writeln!(f, "  {}: {}", problem.path.display(), problem.reason)?;
                }
                Ok(())
            }
            Self::Failed(error) => write!(f, "import failed: {error}"),
        }
    }
}

impl std::error::Error for ImportError {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub live_files: usize,
    pub archive_files: usize,
    /// Rows written per status, in vocabulary order, empty statuses omitted.
    pub inserted: Vec<(TaskStatus, usize)>,
    /// Files whose task was already in the database, byte for byte.
    pub skipped: usize,
    /// Rows that named a product in a form this schema cannot store.
    pub legacy_product_refs: usize,
    /// Products the imported rows name that the catalogue does not carry. A
    /// warning, never a refusal: the catalogue is curated separately, and the
    /// `ready` gate is what asks for it later.
    pub uncatalogued_products: Vec<String>,
}

impl ImportSummary {
    #[must_use]
    pub fn inserted_total(&self) -> usize {
        self.inserted.iter().map(|(_, count)| count).sum()
    }
}

impl fmt::Display for ImportSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "read {} file(s): {} live, {} archive",
            self.live_files + self.archive_files,
            self.live_files,
            self.archive_files
        )?;
        let by_status: Vec<String> = self
            .inserted
            .iter()
            .map(|(status, count)| format!("{} {count}", status.as_str()))
            .collect();
        writeln!(
            f,
            "inserted {}{}{}",
            self.inserted_total(),
            if by_status.is_empty() { "" } else { ": " },
            by_status.join(", ")
        )?;
        writeln!(f, "skipped {} (already imported, unchanged)", self.skipped)?;
        // Deliberately its own line: this is a row with no product at all,
        // which is a different situation from a product the catalogue lacks.
        if self.legacy_product_refs > 0 {
            writeln!(
                f,
                "{} task(s) kept a legacy product reference in the body (product_id left unset)",
                self.legacy_product_refs
            )?;
        }
        if !self.uncatalogued_products.is_empty() {
            writeln!(
                f,
                "warning: {} product(s) not in the catalogue: {}",
                self.uncatalogued_products.len(),
                self.uncatalogued_products.join(", ")
            )?;
        }
        Ok(())
    }
}

/// Read every `*.md` under the given directories and write one task row each.
///
/// All or nothing: everything is parsed and checked first, and a single
/// `IMMEDIATE` transaction writes the lot. One unreadable file, one duplicate
/// id, one status nobody can map, or one row that is already in the database
/// with different content leaves the database exactly as it was.
pub fn import_markdown(
    db: &Db,
    sources: &ImportSources,
    clock: &dyn Clock,
) -> Result<ImportSummary, ImportError> {
    if sources.live.is_none() && sources.archive.is_none() {
        return Err(ImportError::Failed(Error::Invalid(
            "at least one source directory is required".into(),
        )));
    }
    let mut problems = Vec::new();
    let live = markdown_files(sources.live.as_deref(), &mut problems);
    let archive = markdown_files(sources.archive.as_deref(), &mut problems);

    let mut parsed: Vec<Parsed> = Vec::new();
    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();
    for path in live.iter().chain(archive.iter()) {
        match parse_file(path) {
            // The id is the file stem, so the same stem in the live queue and
            // in the archive is two files claiming one row, not a merge.
            Ok(task) if seen.contains_key(&task.id) => problems.push(Problem {
                path: path.clone(),
                reason: format!(
                    "duplicate task id '{}', already read from {}",
                    task.id,
                    seen[&task.id].display()
                ),
            }),
            Ok(task) => {
                seen.insert(task.id.clone(), path.clone());
                parsed.push(task);
            }
            Err(error) => problems.push(Problem {
                path: path.clone(),
                reason: error.to_string(),
            }),
        }
    }
    if !problems.is_empty() {
        return Err(ImportError::Refused(problems));
    }
    write_all(db, &parsed, live.len(), archive.len(), clock.now())
}

/// Every `*.md` under `dir`, recursively and in a stable order, because an
/// archive is usually split into year directories. A directory that cannot be
/// read is itself a problem, so a typo is reported rather than imported as
/// zero files.
fn markdown_files(dir: Option<&Path>, problems: &mut Vec<Problem>) -> Vec<PathBuf> {
    let Some(dir) = dir else {
        return Vec::new();
    };
    let mut files = Vec::new();
    if let Err(error) = walk(dir, &mut files) {
        problems.push(Problem {
            path: dir.to_path_buf(),
            reason: error.to_string(),
        });
        return Vec::new();
    }
    files
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries = fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<PathBuf>, _>>()?;
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.extension().and_then(OsStr::to_str) == Some("md") {
            out.push(path);
        }
    }
    Ok(())
}

/// One markdown file, already checked and ready to become a row.
#[derive(Debug)]
struct Parsed {
    path: PathBuf,
    id: String,
    title: String,
    body: String,
    status: TaskStatus,
    product_id: Option<String>,
    /// The file named a product in a form the column cannot hold, so the value
    /// stayed in the body and the column is empty.
    legacy_product: bool,
    commit_sha: Option<String>,
    verification: Option<String>,
    release_tag: Option<String>,
}

impl Parsed {
    /// Whether the row already in the database is the one this file would
    /// write. Timestamps are excluded on purpose: they say when the import ran,
    /// not what the task is.
    fn matches(&self, existing: &task::Task) -> bool {
        existing.title == self.title
            && existing.body == self.body
            && existing.status == self.status
            && existing.kind == TaskKind::Normal
            && existing.product_id == self.product_id
            && existing.priority == 0
            && existing.commit_sha == self.commit_sha
            && existing.verification == self.verification
            && existing.release_tag == self.release_tag
    }
}

fn parse_file(path: &Path) -> Result<Parsed, Error> {
    let bytes = fs::read(path)?;
    let document = split_document(&bytes)?;
    let frontmatter = &document.frontmatter;

    let id = path
        .file_stem()
        .and_then(OsStr::to_str)
        .ok_or_else(|| Error::Invalid("file name is not usable as a task id".into()))?
        .to_owned();
    let title = get_str(frontmatter, "title")
        .filter(|title| !title.trim().is_empty())
        .ok_or_else(|| Error::Invalid("title is required in the frontmatter".into()))?;
    let raw_status = get_str(frontmatter, "status")
        .ok_or_else(|| Error::Invalid("status is required in the frontmatter".into()))?;
    let status = map_status(&raw_status)?;
    let product = product_reference(frontmatter);
    let body =
        std::str::from_utf8(&document.body).map_err(|err| Error::Invalid(err.to_string()))?;

    Ok(Parsed {
        path: path.to_path_buf(),
        id,
        title,
        body: fold_metadata(body, frontmatter, &raw_status, product.stored_key)?,
        status,
        product_id: product.id,
        legacy_product: product.legacy,
        commit_sha: get_str(frontmatter, "commit_sha"),
        verification: get_str(frontmatter, "verification"),
        release_tag: get_str(frontmatter, "release_tag"),
    })
}

/// What a file says about its product.
struct ProductReference {
    /// The value for the `product_id` column, when there is one to store.
    id: Option<String>,
    /// The frontmatter key the stored value came from, so the fold knows which
    /// key not to repeat. `None` when nothing reached the column.
    stored_key: Option<&'static str>,
    /// The file named a product, but not as `org/repo`.
    legacy: bool,
}

/// Read the product this file names, and decide whether it can be stored.
///
/// A reference that is not `org/repo` predates the convention, and this is an
/// import of history: refusing it would mean editing an archive to migrate it,
/// which v0.2.0 explicitly does not ask for. So the column stays empty, the
/// original value stays in the folded block, and the decision moves to the
/// person who promotes the task — `ready` refuses a task without a catalogued
/// product with `product_required`, which is where the real product gets
/// chosen. Live and archive are treated identically; there is one rule.
fn product_reference(frontmatter: &Mapping) -> ProductReference {
    // v0.1 named the product `target_space`; later files used `product_id`.
    let named = ["target_space", "product_id"].into_iter().find_map(|key| {
        get_str(frontmatter, key)
            .filter(|value| !value.trim().is_empty())
            .map(|value| (key, value))
    });
    match named {
        Some((key, value)) if check_product_id("product id", &value).is_ok() => ProductReference {
            id: Some(value),
            stored_key: Some(key),
            legacy: false,
        },
        Some(_) => ProductReference {
            id: None,
            stored_key: None,
            legacy: true,
        },
        None => ProductReference {
            id: None,
            stored_key: None,
            legacy: false,
        },
    }
}

/// The v0.1 status this row came from, in the v0.2 vocabulary.
///
/// Anything [`RENAMED`] does not cover kept its name, so the domain parser is
/// the authority on it and this never restates the vocabulary. A status neither
/// knows is an error: a queue that silently lost a row would be worse than one
/// that refused to move.
fn map_status(raw: &str) -> Result<TaskStatus, Error> {
    if let Some((_, status)) = RENAMED.iter().find(|(from, _)| *from == raw) {
        return Ok(*status);
    }
    TaskStatus::parse(raw).map_err(|_| Error::Invalid(format!("unknown v0.1 status '{raw}'")))
}

/// Append the frontmatter the schema has no column for to the body, as one
/// marked YAML block.
///
/// The block is machine-findable on purpose: heading and fence are fixed, and
/// the values keep their YAML types, so a later reader can lift the block back
/// out. The original status leads it, always, even when nothing else is left
/// over — that is what records where the row came from. `stored_key` is the
/// product key that did reach its column, if any; a product reference the
/// column could not hold is left over like everything else, so no value the
/// file carried is dropped.
fn fold_metadata(
    body: &str,
    frontmatter: &Mapping,
    raw_status: &str,
    stored_key: Option<&str>,
) -> Result<String, Error> {
    let mut leftover = Mapping::new();
    leftover.insert(
        Value::String("status".into()),
        Value::String(raw_status.to_owned()),
    );
    for (key, value) in frontmatter {
        let mapped = matches!(key, Value::String(name)
            if MAPPED_KEYS.contains(&name.as_str()) || stored_key == Some(name.as_str()));
        if !mapped {
            leftover.insert(key.clone(), value.clone());
        }
    }

    let mut out = body.trim_end().to_owned();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(BLOCK_HEADING);
    out.push_str(&serialize_mapping(&leftover)?);
    out.push_str("```\n");
    Ok(out)
}

/// Write every parsed task in one transaction, or none of them.
fn write_all(
    db: &Db,
    parsed: &[Parsed],
    live_files: usize,
    archive_files: usize,
    now: OffsetDateTime,
) -> Result<ImportSummary, ImportError> {
    let stamp = format_z(now);
    // `with_tx` commits whenever the closure returns `Ok`, so a conflict has to
    // leave through `Err` to roll the transaction back. The conflicts
    // themselves travel out here, where the caller can still read them.
    let mut conflicts: Vec<Problem> = Vec::new();
    let written = db.with_tx(|tx| {
        let mut counts = [0usize; ALL_STATUSES.len()];
        let mut skipped = 0;
        for task in parsed {
            match task::read(tx, &task.id) {
                Ok(existing) if task.matches(&existing) => skipped += 1,
                Ok(_) => conflicts.push(Problem {
                    path: task.path.clone(),
                    reason: format!(
                        "task '{}' is already in the database with different content",
                        task.id
                    ),
                }),
                Err(Error::NotFound) => {
                    insert(tx, task, &stamp)?;
                    counts[status_index(task.status)] += 1;
                }
                Err(other) => return Err(other),
            }
        }
        if !conflicts.is_empty() {
            return Err(Error::Conflict(
                "the import disagrees with rows already in the database".into(),
            ));
        }
        Ok(ImportSummary {
            live_files,
            archive_files,
            inserted: ALL_STATUSES
                .into_iter()
                .zip(counts)
                .filter(|(_, count)| *count > 0)
                .collect(),
            skipped,
            legacy_product_refs: parsed.iter().filter(|task| task.legacy_product).count(),
            uncatalogued_products: uncatalogued_products(tx, parsed)?,
        })
    });
    match written {
        Ok(summary) => Ok(summary),
        Err(_) if !conflicts.is_empty() => Err(ImportError::Refused(conflicts)),
        Err(error) => Err(ImportError::Failed(error)),
    }
}

fn insert(tx: &Connection, task: &Parsed, stamp: &str) -> Result<(), Error> {
    // A row imported already past `done` earned that status before this
    // database existed, so `stamp` — the import's own timestamp — is the same
    // best-effort estimate [`SCHEMA_V13`] backfills for a pre-existing row. A
    // row imported short of `done` gets no `done_at`, for the same reason the
    // migration gives none: guessing one would assert a completion that never
    // happened.
    let done_at = matches!(
        task.status,
        TaskStatus::Done | TaskStatus::Approved | TaskStatus::Merged | TaskStatus::Released
    )
    .then_some(stamp);
    tx.execute(
        "INSERT INTO tasks (id, title, body, status, kind, product_id, priority,
                            commit_sha, verification, release_tag, created_at, updated_at,
                            done_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9, ?10, ?10, ?11)",
        rusqlite::params![
            task.id,
            task.title,
            task.body,
            task.status.as_str(),
            TaskKind::Normal.as_str(),
            task.product_id,
            task.commit_sha,
            task.verification,
            task.release_tag,
            stamp,
            done_at,
        ],
    )?;
    Ok(())
}

/// The products the import names that the catalogue does not carry. The
/// migration keeps the snapshot as it was written; inventing catalogue entries
/// from it would register products nobody curated.
fn uncatalogued_products(tx: &Connection, parsed: &[Parsed]) -> Result<Vec<String>, Error> {
    let named: BTreeSet<&str> = parsed
        .iter()
        .filter_map(|task| task.product_id.as_deref())
        .collect();
    let mut missing = Vec::new();
    for product_id in named {
        match product::read(tx, product_id) {
            Ok(_) => {}
            Err(Error::NotFound) => missing.push(product_id.to_owned()),
            Err(other) => return Err(other),
        }
    }
    Ok(missing)
}

fn status_index(status: TaskStatus) -> usize {
    ALL_STATUSES
        .iter()
        .position(|candidate| *candidate == status)
        .expect("every status is in the vocabulary")
}

#[cfg(test)]
mod tests {
    use serde_norway::{Mapping, Value};

    use super::{ImportSources, fold_metadata, map_status, product_reference};
    use crate::task::TaskStatus;

    fn frontmatter(pairs: &[(&str, &str)]) -> Mapping {
        let mut map = Mapping::new();
        for (key, value) in pairs {
            map.insert(
                Value::String((*key).to_owned()),
                Value::String((*value).to_owned()),
            );
        }
        map
    }

    /// The mapping table, written out. `done → merged` and `awaiting_user →
    /// done` are the two that carry a decision rather than a rename, and an
    /// unknown status is refused rather than dropped.
    #[test]
    fn every_v0_1_status_maps_to_one_of_the_v0_2_vocabulary() {
        let table = [
            ("draft", TaskStatus::Draft),
            ("ready", TaskStatus::Ready),
            ("running", TaskStatus::Wip),
            ("awaiting_user", TaskStatus::Done),
            ("done", TaskStatus::Merged),
            ("release_requested", TaskStatus::Merged),
            ("release_failed", TaskStatus::Merged),
            ("released", TaskStatus::Released),
            ("blocked", TaskStatus::Blocked),
            ("cancelled", TaskStatus::Cancelled),
            ("dropped", TaskStatus::Dropped),
        ];
        for (from, to) in table {
            assert_eq!(map_status(from).expect(from), to, "{from}");
        }
        assert_ne!(
            map_status("done").unwrap(),
            TaskStatus::Done,
            "v0.1 done was accepted work, and mapping it to done would raise it \
             as a merge candidate again"
        );
        for unknown in ["frobnicated", "", "Draft", "awaiting user"] {
            assert!(map_status(unknown).is_err(), "{unknown:?} must be refused");
        }
    }

    /// Only `org/repo` reaches the column. Anything else was written before the
    /// convention, and the import keeps it in the body rather than refusing an
    /// archive it is not allowed to rewrite.
    #[test]
    fn a_product_reference_reaches_its_column_only_as_org_repo() {
        let stored = product_reference(&frontmatter(&[("target_space", "example/repo")]));
        assert_eq!(stored.id.as_deref(), Some("example/repo"));
        assert_eq!(stored.stored_key, Some("target_space"));
        assert!(!stored.legacy);

        let fallback = product_reference(&frontmatter(&[("product_id", "example/repo")]));
        assert_eq!(fallback.id.as_deref(), Some("example/repo"));
        assert_eq!(fallback.stored_key, Some("product_id"));

        for value in ["tasks", "projects/queue/tasks", "../etc", "a\\b"] {
            let legacy = product_reference(&frontmatter(&[("target_space", value)]));
            assert!(legacy.id.is_none(), "{value} must not reach the column");
            assert!(
                legacy.stored_key.is_none(),
                "{value} stays in the folded block, so no key is consumed"
            );
            assert!(legacy.legacy, "{value} must be counted as legacy");
        }

        let absent = product_reference(&frontmatter(&[("title", "t")]));
        assert!(absent.id.is_none());
        assert!(
            !absent.legacy,
            "naming no product at all is not a legacy reference"
        );
        let blank = product_reference(&frontmatter(&[("target_space", "  ")]));
        assert!(!blank.legacy, "an empty value names nothing");
    }

    #[test]
    fn the_folded_block_keeps_the_original_status_and_every_unmapped_value() {
        let mut frontmatter = Mapping::new();
        frontmatter.insert(Value::String("title".into()), Value::String("t".into()));
        frontmatter.insert(Value::String("status".into()), Value::String("done".into()));
        frontmatter.insert(
            Value::String("target_space".into()),
            Value::String("tasks".into()),
        );
        frontmatter.insert(
            Value::String("tags".into()),
            Value::Sequence(vec![Value::String("a".into())]),
        );

        let body = fold_metadata("Original.\n", &frontmatter, "done", None).expect("fold");
        assert!(
            body.contains("target_space: tasks"),
            "a reference the column could not hold stays in the block: {body}"
        );
        let mapped =
            fold_metadata("Original.\n", &frontmatter, "done", Some("target_space")).expect("fold");
        assert!(
            !mapped.contains("target_space"),
            "a reference that reached the column is not repeated: {mapped}"
        );
        assert!(
            body.starts_with("Original.\n\n---\n\n## Imported v0.1 metadata\n"),
            "{body}"
        );
        assert!(body.contains("status: done"), "{body}");
        assert!(body.contains("tags:\n- a\n"), "a list stays a list: {body}");
        assert!(
            !body.contains("title:"),
            "a mapped key is not repeated: {body}"
        );

        let empty = fold_metadata("", &frontmatter, "done", None).expect("fold");
        assert!(
            empty.starts_with("---\n\n## Imported v0.1 metadata\n"),
            "an empty body still records where the row came from: {empty}"
        );
    }

    #[test]
    fn arguments_name_at_least_one_directory() {
        let args: Vec<String> = ["--live", "queue"]
            .iter()
            .map(|a| (*a).to_owned())
            .collect();
        let sources = ImportSources::from_args(&args).expect("live alone is enough");
        assert_eq!(sources.live.as_deref(), Some(std::path::Path::new("queue")));
        assert!(sources.archive.is_none());
        assert!(ImportSources::from_args(&[]).is_err());
    }
}
