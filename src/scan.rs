//! The product catalogue, derived from the project tree on disk.
//!
//! A roster maintained by hand drifts: a product is deleted locally and stays in
//! the catalogue forever. The filesystem already knows which products exist, so
//! this reads them instead — the directories under `APP_PROJECTS_DIR`, two levels
//! deep, as `<org>/<repo>`.
//!
//! Read-only, and git-binary-free. Everything derived here comes out of files a
//! clone already has: `.git/config` for the remote, `README.md` for the
//! description, and a `.github/workflows` directory for whether the product
//! releases. Shelling out to `git` would need a writable `HOME` and a git binary,
//! neither of which a read-only container mount has.
//!
//! The tree is also the boundary. Every path this opens is canonicalised and has
//! to sit under the canonical root: a `.git` that is a symlink, a `gitdir:`
//! pointer that is absolute or climbs out with `..`, a worktree `commondir`, a
//! linked `README.md`, a linked `.github/workflows` — each of those is a way to
//! make the walk read a file the operator never put in the tree. What leaves the
//! root is skipped and counted, not followed.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::product::{Product, check_product_id};

/// Names the root of the `<org>/<repo>` tree the catalogue is derived from.
const PROJECTS_DIR: &str = "APP_PROJECTS_DIR";

/// The retired roster variable. A deployment still setting it is refused rather
/// than started with the roster silently ignored.
const PRODUCTS_SEED: &str = "APP_PRODUCTS_SEED";

/// Where the catalogue comes from on this start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Catalogue {
    /// No tree configured: the catalogue is whatever the API put there.
    Curated,
    /// Derived from the project tree rooted at this path.
    Derived(PathBuf),
}

/// Why one entry of the tree is not a product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// A directory with no `.git` at all, or a `.git` naming a directory that is
    /// not there.
    NotARepository,
    /// A `.git` that is not a git directory: no `HEAD` that reads as a ref, no
    /// object store, or no `refs`. A stray `config` is not a clone.
    IncompleteRepository,
    /// A `.git`, a `gitdir:` pointer, or a `commondir` that resolves outside the
    /// root. The tree the catalogue is derived from is the only thing read.
    OutsideRoot,
    /// A repository whose config names no `origin` remote.
    NoOrigin,
    /// A name that is not a legal product id segment.
    InvalidId,
    /// A symbolic link, at either level.
    Symlink,
}

impl SkipReason {
    /// The stable slug used in the startup log.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotARepository => "not_a_repository",
            Self::IncompleteRepository => "incomplete_repository",
            Self::OutsideRoot => "outside_root",
            Self::NoOrigin => "no_origin",
            Self::InvalidId => "invalid_id",
            Self::Symlink => "symlink",
        }
    }
}

/// One entry the scan looked at and did not turn into a product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The entry as it appears under the root, `org` or `org/repo`.
    pub name: String,
    pub reason: SkipReason,
}

/// What one walk of the tree found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// The products, ordered by id.
    pub products: Vec<Product>,
    pub skipped: Vec<Skipped>,
}

impl Report {
    /// How many entries each reason accounts for, for one summary log line.
    #[must_use]
    pub fn skipped_by_reason(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for skipped in &self.skipped {
            *counts.entry(skipped.reason.as_str()).or_default() += 1;
        }
        counts
    }
}

/// Decide from the environment where the catalogue comes from.
///
/// # Errors
///
/// Returns `Error::Invalid` when the retired `APP_PRODUCTS_SEED` is still set.
/// Ignoring it would start a server whose catalogue disagrees with the roster the
/// operator is still maintaining, and say nothing about it.
pub fn source_from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Catalogue, Error> {
    let set = |name: &str| get(name).filter(|value| !value.is_empty());
    if set(PRODUCTS_SEED).is_some() {
        return Err(Error::Invalid(format!(
            "{PRODUCTS_SEED} is retired: the catalogue is derived from the project tree now, so unset it and point {PROJECTS_DIR} at the directory holding <org>/<repo>"
        )));
    }
    Ok(set(PROJECTS_DIR).map_or(Catalogue::Curated, |root| {
        Catalogue::Derived(PathBuf::from(root))
    }))
}

/// Walk `root` two levels deep and derive a product from every git repository.
///
/// Nothing outside `root` is read: see [`Walk`] for the boundary and the range of
/// metadata a repository inside it may keep.
///
/// # Errors
///
/// Returns `Error::Io` when the root, an org directory, or a file a repository
/// does have cannot be read. A repository that simply lacks a remote, a README,
/// or workflows is skipped or defaulted, never an error: only a tree that could
/// not be read at all is, because an empty answer reads as "every product is
/// gone".
pub fn scan(root: impl AsRef<Path>) -> Result<Report, Error> {
    Walk::rooted(root.as_ref())?.run()
}

/// One walk of the tree, and the boundary it is allowed to read inside.
///
/// The allowed range is the canonical root and everything under it. A repository
/// keeps its metadata there too: a plain clone in `<repo>/.git`, a worktree or a
/// submodule in the `gitdir:` the pointer file names *provided that directory is
/// also under the root* — which is where git itself puts them, inside the
/// superproject's `.git/modules` or `.git/worktrees`. A pointer, a link, or a
/// `commondir` that leads anywhere else is skipped as [`SkipReason::OutsideRoot`]
/// and appears in the startup log's per-reason counts.
struct Walk {
    /// The canonical root every resolved path has to sit under.
    root: PathBuf,
}

/// Where a path led once every link in it had been followed.
enum Inside {
    /// The canonical path, which sits under the root.
    Yes(PathBuf),
    /// It resolved outside the root: readable in principle, off limits here.
    No,
    /// Nothing is there.
    Missing,
}

/// The two directories a repository's metadata lives in. They are the same
/// directory for a plain clone, and differ for a worktree, whose `HEAD` is its
/// own while config and refs are shared.
struct Dirs {
    git: PathBuf,
    common: PathBuf,
}

/// What `.git` turned out to name.
enum Located {
    At(Dirs),
    /// No `.git` here, or one naming a directory that is not there.
    Absent,
    /// It resolves outside the root.
    Outside,
}

/// One hop of [`Walk::locate`]: a path that has to be a directory inside the
/// root.
enum Hop {
    Dir(PathBuf),
    /// Not there, or there but not a directory.
    Absent,
    /// Outside the root.
    Outside,
}

impl Walk {
    fn rooted(root: &Path) -> Result<Self, Error> {
        // Canonicalising the root is what makes the comparison meaningful: the
        // configured root may itself be reached through a symlink, and every
        // path checked against it is canonical too. A root that is not there
        // fails here rather than reporting an empty tree.
        let canonical = fs::canonicalize(root).map_err(|err| io_error(root, &err))?;
        Ok(Self { root: canonical })
    }

    fn run(&self) -> Result<Report, Error> {
        let mut report = Report::default();
        for org in entries(&self.root)? {
            if org.is_symlink {
                report.skipped.push(Skipped {
                    name: org.name,
                    reason: SkipReason::Symlink,
                });
                continue;
            }
            if !org.is_dir {
                continue;
            }
            for repo in entries(&org.path)? {
                let name = format!("{}/{}", org.name, repo.name);
                match self.look_at(&name, &repo)? {
                    Outcome::Product(product) => report.products.push(product),
                    Outcome::Skip(reason) => report.skipped.push(Skipped { name, reason }),
                    Outcome::Ignore => {}
                }
            }
        }
        report
            .products
            .sort_by(|left, right| left.id.cmp(&right.id));
        Ok(report)
    }

    fn look_at(&self, id: &str, entry: &Entry) -> Result<Outcome, Error> {
        if entry.is_symlink {
            return Ok(Outcome::Skip(SkipReason::Symlink));
        }
        if !entry.is_dir {
            return Ok(Outcome::Ignore);
        }
        if check_product_id("id", id).is_err() {
            return Ok(Outcome::Skip(SkipReason::InvalidId));
        }
        let dirs = match self.locate(&entry.path)? {
            Located::At(dirs) => dirs,
            Located::Absent => return Ok(Outcome::Skip(SkipReason::NotARepository)),
            Located::Outside => return Ok(Outcome::Skip(SkipReason::OutsideRoot)),
        };
        if !self.is_git_directory(&dirs)? {
            return Ok(Outcome::Skip(SkipReason::IncompleteRepository));
        }
        let Some(config) = self.read_within(&dirs.common.join("config"))? else {
            return Ok(Outcome::Skip(SkipReason::NoOrigin));
        };
        let Some(url) = origin_url(&config) else {
            return Ok(Outcome::Skip(SkipReason::NoOrigin));
        };
        let readme = self.read_within(&entry.path.join("README.md"))?;
        Ok(Outcome::Product(Product {
            id: id.to_owned(),
            repository: normalize_remote_url(url),
            description: readme
                .as_deref()
                .map(first_heading_line)
                .unwrap_or_default(),
            releases: self.releases(&entry.path)?,
            // On disk is what the walk means, so nothing it finds is archived.
            archived: false,
        }))
    }

    /// The directories holding this repository's `HEAD`, config and refs.
    ///
    /// A plain clone keeps them in `.git`. A worktree and a submodule keep `.git`
    /// as a *file* naming the real git directory, and a worktree's git directory
    /// defers to the `commondir` beside it for everything shared — which is where
    /// config and tags are. Reading `<repo>/.git/config` directly would report
    /// both as having no remote, and drop them from the catalogue.
    ///
    /// Every hop is resolved against the root, so following a pointer can never
    /// walk out of the tree.
    fn locate(&self, repo: &Path) -> Result<Located, Error> {
        let dot_git = repo.join(".git");
        let meta = match fs::symlink_metadata(&dot_git) {
            Ok(meta) => meta,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Located::Absent),
            Err(err) => return Err(io_error(&dot_git, &err)),
        };
        // A pointer file names the git directory; anything else *is* it, whether
        // a directory or a link to one — resolving decides which, and whether it
        // is still inside the tree.
        let named = if meta.is_file() {
            let Some(pointer) = self
                .read_within(&dot_git)?
                .as_deref()
                .and_then(gitdir_pointer)
                .map(str::to_owned)
            else {
                return Ok(Located::Absent);
            };
            // A pointer may be absolute or relative to the working tree; `join`
            // handles both, and the resolve below is what rules out the ones
            // leading elsewhere.
            repo.join(pointer)
        } else {
            dot_git
        };
        let git = match self.dir_within(&named)? {
            Hop::Dir(real) => real,
            Hop::Absent => return Ok(Located::Absent),
            Hop::Outside => return Ok(Located::Outside),
        };
        let common = match self.read_within(&git.join("commondir"))? {
            Some(relative) if !relative.trim().is_empty() => {
                match self.dir_within(&git.join(relative.trim()))? {
                    Hop::Dir(real) => real,
                    Hop::Absent => return Ok(Located::Absent),
                    Hop::Outside => return Ok(Located::Outside),
                }
            }
            _ => git.clone(),
        };
        Ok(Located::At(Dirs { git, common }))
    }

    /// A path that has to resolve to a directory inside the root.
    fn dir_within(&self, path: &Path) -> Result<Hop, Error> {
        match self.resolve(path)? {
            Inside::Yes(real) if real.is_dir() => Ok(Hop::Dir(real)),
            Inside::Yes(_) | Inside::Missing => Ok(Hop::Absent),
            Inside::No => Ok(Hop::Outside),
        }
    }

    /// Whether this really is a git directory, rather than a directory named
    /// `.git`.
    ///
    /// git's own test, which is deliberately not "it has a config": a `HEAD` that
    /// reads as a ref, an object store, and a `refs` directory. A hand-made or
    /// stray `.git/config` carrying a remote would otherwise mint a product for a
    /// directory no clone ever made, and its `id` would name a working copy
    /// nobody can check out.
    ///
    /// `HEAD` is the worktree's own; the object store and refs are shared, so
    /// they are looked for in the common directory.
    fn is_git_directory(&self, dirs: &Dirs) -> Result<bool, Error> {
        let Some(head) = self.read_within(&dirs.git.join("HEAD"))? else {
            return Ok(false);
        };
        if !head_names_a_commit(&head) {
            return Ok(false);
        }
        for shared in ["objects", "refs"] {
            if !matches!(self.dir_within(&dirs.common.join(shared))?, Hop::Dir(_)) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Whether this product ships releases: whether it has a
    /// `.github/workflows` directory.
    ///
    /// The directory is the whole test — no name is read, and an empty one still
    /// counts. A product that releases does not make its users build it: a Rust
    /// binary is compiled by CI, a service is shipped as an image CI pushes. Both
    /// mean workflows, so the directory is the shape of a repository whose
    /// releases are built for it, and what the workflows happen to be called is
    /// not a fact about the product.
    ///
    /// Only a directory answers yes: a file of that name is not a workflow
    /// directory, and one that resolves outside the root is not this tree's.
    fn releases(&self, repo: &Path) -> Result<bool, Error> {
        Ok(matches!(
            self.dir_within(&repo.join(".github/workflows"))?,
            Hop::Dir(_)
        ))
    }

    /// The canonical form of `path`, refused when it leaves the root.
    ///
    /// `canonicalize` is what follows the links — including a chain of them — so
    /// the comparison happens on the file that would actually be opened rather
    /// than on the name asking for it.
    fn resolve(&self, path: &Path) -> Result<Inside, Error> {
        match fs::canonicalize(path) {
            Ok(real) if real.starts_with(&self.root) => Ok(Inside::Yes(real)),
            Ok(_) => Ok(Inside::No),
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Ok(Inside::Missing)
            }
            Err(err) => Err(io_error(path, &err)),
        }
    }

    /// A file a repository may not have, read only when it is inside the root.
    ///
    /// Absent and out-of-tree are both `None`: a `README.md` linked to somewhere
    /// else is a product with no description, not a product described by a file
    /// the operator never put in the tree. Present, in the tree, and unreadable
    /// stays an error, because that is a fault rather than a missing file.
    fn read_within(&self, path: &Path) -> Result<Option<String>, Error> {
        match self.resolve(path)? {
            Inside::Yes(real) => match fs::read_to_string(&real) {
                Ok(text) => Ok(Some(text)),
                Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(err) => Err(io_error(&real, &err)),
            },
            Inside::No | Inside::Missing => Ok(None),
        }
    }
}

/// What one entry at the repository level turned out to be.
enum Outcome {
    Product(Product),
    Skip(SkipReason),
    /// Not a candidate at all — a stray file rather than a project directory.
    Ignore,
}

/// One directory entry, classified without following links.
struct Entry {
    path: PathBuf,
    /// The file name, lossily decoded. A name that needed replacing cannot be a
    /// legal product id anyway, so the check downstream refuses it.
    name: String,
    is_dir: bool,
    is_symlink: bool,
}

fn entries(dir: &Path) -> Result<Vec<Entry>, Error> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).map_err(|err| io_error(dir, &err))? {
        let entry = entry.map_err(|err| io_error(dir, &err))?;
        let path = entry.path();
        let kind = entry.file_type().map_err(|err| io_error(&path, &err))?;
        found.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path,
            is_dir: kind.is_dir(),
            is_symlink: kind.is_symlink(),
        });
    }
    found.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(found)
}

fn gitdir_pointer(text: &str) -> Option<&str> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("gitdir:"))
        .map(str::trim)
        .filter(|pointer| !pointer.is_empty())
}

/// Whether `HEAD` reads as git writes it: a symbolic ref into `refs/`, or a
/// detached object name. A `.git` whose `HEAD` says anything else was not made
/// by git.
fn head_names_a_commit(head: &str) -> bool {
    let head = head.trim();
    if let Some(target) = head.strip_prefix("ref:") {
        return target.trim().starts_with("refs/");
    }
    // sha-1 and sha-256 object names, the two git has.
    matches!(head.len(), 40 | 64) && head.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The `origin` remote's URL, or `None` when the config names no origin.
///
/// Only `origin` counts. A fork whose `upstream` is listed first must not report
/// the upstream as its repository.
fn origin_url(config: &str) -> Option<&str> {
    let mut in_origin = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_origin = line.split_whitespace().collect::<String>() == "[remote\"origin\"]";
            continue;
        }
        if in_origin
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "url"
            && !value.trim().is_empty()
        {
            return Some(value.trim());
        }
    }
    None
}

/// The browsable form of a remote URL.
///
/// One repository has several spellings — `git@host:org/repo.git`,
/// `ssh://git@host/org/repo.git`, `https://host/org/repo` — and the catalogue
/// stores a link a human can open, so they all land on the same value.
fn normalize_remote_url(raw: &str) -> String {
    let raw = raw.trim();
    let https = if let Some(rest) = raw.strip_prefix("ssh://") {
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        format!("https://{}/{path}", host_of(authority))
    } else if !raw.contains("://")
        && let Some((authority, path)) = raw.split_once(':')
    {
        format!(
            "https://{}/{}",
            host_of(authority),
            path.trim_start_matches('/')
        )
    } else {
        raw.to_owned()
    };
    let trimmed = https.strip_suffix('/').unwrap_or(&https);
    trimmed.strip_suffix(".git").unwrap_or(trimmed).to_owned()
}

fn host_of(authority: &str) -> &str {
    authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
}

/// The README headline, with its markdown taken off. No README is an empty
/// description, not a missing product.
fn first_heading_line(readme: &str) -> String {
    readme
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_owned())
        .unwrap_or_default()
}

fn io_error(path: &Path, err: &io::Error) -> Error {
    Error::Io(format!("{}: {err}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        Catalogue, Report, first_heading_line, normalize_remote_url, origin_url, scan,
        source_from_vars,
    };
    use crate::error::Error;

    /// The three things git's own repository test looks for. A `.git` without
    /// them is a directory named `.git`, and the scan has to say so.
    fn git_dir(path: &Path) {
        fs::create_dir_all(path.join("objects")).unwrap();
        fs::create_dir_all(path.join("refs")).unwrap();
        fs::write(path.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    }

    /// A repository the scan should accept: a `.git` directory git would
    /// recognise, a config naming `origin`, and nothing else unless the test adds
    /// it.
    fn repo(root: &Path, id: &str, origin: &str) -> PathBuf {
        let dir = root.join(id);
        git_dir(&dir.join(".git"));
        if !origin.is_empty() {
            fs::write(
                dir.join(".git/config"),
                format!("[core]\n\tbare = false\n[remote \"origin\"]\n\turl = {origin}\n"),
            )
            .unwrap();
        }
        dir
    }

    fn write(path: PathBuf, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn ids(report: &Report) -> Vec<&str> {
        report.products.iter().map(|p| p.id.as_str()).collect()
    }

    fn skips(report: &Report) -> Vec<(&str, &'static str)> {
        report
            .skipped
            .iter()
            .map(|s| (s.name.as_str(), s.reason.as_str()))
            .collect()
    }

    /// A root with a sibling directory beside it, so a test can point at
    /// something that is definitely outside the tree by a path it can also spell
    /// relatively.
    fn root_and_outside() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("projects");
        let outside = base.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        (base, root, outside)
    }

    /// The id is where the product sits locally, and the repository is where git
    /// pushes it. An ssh remote is the same repository as its browsable https
    /// URL, so both spellings have to land on one value — otherwise a clone made
    /// over ssh and one made over https are two products.
    #[test]
    fn a_remote_url_is_normalized_to_its_browsable_form() {
        for (raw, expected) in [
            (
                "git@github.com:miyabisun/5ch-viewer.git",
                "https://github.com/miyabisun/5ch-viewer",
            ),
            ("git@github.com:org/repo", "https://github.com/org/repo"),
            (
                "https://github.com/org/repo.git",
                "https://github.com/org/repo",
            ),
            ("https://github.com/org/repo", "https://github.com/org/repo"),
            (
                "  https://github.com/org/repo.git/  ",
                "https://github.com/org/repo",
            ),
            (
                "ssh://git@github.com/org/repo.git",
                "https://github.com/org/repo",
            ),
        ] {
            assert_eq!(normalize_remote_url(raw), expected, "{raw}");
        }
    }

    /// The description is the README headline, with the markdown taken off. A
    /// README that opens with blank lines or a badge line still has to answer
    /// with prose, and no README at all is not an error.
    #[test]
    fn the_description_is_the_first_readme_line_without_its_hashes() {
        for (readme, expected) in [
            ("# Task Server\n\nprose follows\n", "Task Server"),
            ("\n\n   ## Deeper heading\n", "Deeper heading"),
            ("no heading at all\nsecond line\n", "no heading at all"),
            ("", ""),
            ("\n \n\t\n", ""),
        ] {
            assert_eq!(first_heading_line(readme), expected, "{readme:?}");
        }
    }

    /// Only `origin` decides the repository. A config with an `upstream` remote
    /// listed first must not hand over the fork's URL, and a config with no
    /// origin at all reports nothing rather than guessing.
    #[test]
    fn only_the_origin_remote_names_the_repository() {
        let config = "[remote \"upstream\"]\n\turl = https://github.com/other/repo\n\
                      [remote \"origin\"]\n\turl = git@github.com:org/repo.git\n\
                      [branch \"main\"]\n\tremote = origin\n";
        assert_eq!(origin_url(config), Some("git@github.com:org/repo.git"));

        assert_eq!(
            origin_url("[remote \"upstream\"]\n\turl = https://github.com/other/repo\n"),
            None
        );
        assert_eq!(origin_url("[core]\n\tbare = false\n"), None);
        assert_eq!(origin_url(""), None);
    }

    /// `releases` asks the clone one question: is there a `.github/workflows`
    /// directory? A product that releases has its artefacts built for it — a
    /// compiled binary nobody should have to build themselves, an image pushed
    /// to a registry — and that is a workflow, whatever the files in it are
    /// called. So the directory is the whole evidence, an empty one included: it
    /// is the shape of a repository whose releases are made by CI.
    ///
    /// A tag is not evidence. It records that a version was cut, by hand as
    /// easily as by CI, and a repository that tags without a workflow is
    /// released by whoever is at the keyboard rather than by this control plane.
    #[test]
    fn a_workflows_directory_is_the_whole_release_test() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();

        // The name of the workflow says nothing: a repository with any CI at all
        // is one whose releases are built for it.
        let unrelated = repo(root, "org/unrelated-name", "git@github.com:org/a.git");
        write(unrelated.join(".github/workflows/ci.yml"), "on: push\n");

        let empty = repo(root, "org/empty-workflows", "git@github.com:org/b.git");
        fs::create_dir_all(empty.join(".github/workflows")).unwrap();

        let bare = repo(root, "org/no-workflows", "git@github.com:org/c.git");
        fs::create_dir_all(bare.join(".github")).unwrap();

        // Tagged to the eyeballs, and it still does not release.
        let tagged = repo(root, "org/tagged", "git@github.com:org/d.git");
        write(tagged.join(".git/refs/tags/v1.2.3"), "abc\n");
        write(tagged.join(".git/packed-refs"), "abc123 refs/tags/v2.0.0\n");

        // A file where the directory should be is not a directory.
        let filed = repo(root, "org/workflows-file", "git@github.com:org/e.git");
        write(filed.join(".github/workflows"), "not a directory\n");

        let report = scan(root).unwrap();
        assert_eq!(
            report
                .products
                .iter()
                .map(|product| (product.id.as_str(), product.releases))
                .collect::<Vec<_>>(),
            [
                ("org/empty-workflows", true),
                ("org/no-workflows", false),
                ("org/tagged", false),
                ("org/unrelated-name", true),
                ("org/workflows-file", false),
            ],
            "{:?}",
            report.skipped
        );
    }

    /// The tree answers every walk, not the first one. A clone that grows a
    /// workflow directory releases from then on, and one that loses it stops —
    /// which is what makes putting a clone back the whole remedy for a product
    /// that left.
    #[test]
    fn the_release_flag_is_re_read_on_every_walk() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let one = repo(root, "org/one", "git@github.com:org/one.git");

        let releases = |root: &Path| scan(root).unwrap().products[0].releases;
        assert!(!releases(root), "no workflows yet");

        fs::create_dir_all(one.join(".github/workflows")).unwrap();
        assert!(releases(root), "the walk reads the tree as it is now");

        fs::remove_dir_all(one.join(".github")).unwrap();
        assert!(!releases(root), "and again when it goes away");
    }

    /// The whole contract of one walk: two levels, git repositories only, with
    /// every one of the four fields derived from a file the clone already has.
    #[test]
    fn the_tree_becomes_the_catalogue_two_levels_down() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();

        let one = repo(root, "sunny-side/one", "git@github.com:miyabisun/one.git");
        write(one.join("README.md"), "# the first product\n\nbody\n");
        write(one.join(".github/workflows/release.yml"), "on: push\n");

        let two = repo(root, "sunny-side/two", "https://github.com/org/two.git");
        write(two.join(".git/refs/tags/v0.3.1"), "abc123\n");
        // Tagged, and with no workflows: releases stay off.

        let report = scan(root).unwrap();
        assert_eq!(ids(&report), ["sunny-side/one", "sunny-side/two"]);

        let first = &report.products[0];
        assert_eq!(
            first.repository, "https://github.com/miyabisun/one",
            "the local org never rewrites the remote: id and repository are two facts"
        );
        assert_eq!(first.description, "the first product");
        assert!(first.releases, "a workflow directory means it releases");

        let second = &report.products[1];
        assert_eq!(second.repository, "https://github.com/org/two");
        assert_eq!(second.description, "", "no README is an empty description");
        assert!(
            !second.releases,
            "a tag is not a release pipeline: nothing here builds anything"
        );
        assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    }

    /// Everything the walk refuses, and the reason it reports for each. A
    /// directory that is not a repository, a repository with no remote, a name
    /// no product id may take, and a symlink at either level.
    #[test]
    fn entries_that_are_not_products_are_skipped_with_their_reason() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();

        repo(root, "org/kept", "git@github.com:org/kept.git");
        fs::create_dir_all(root.join("org/not-a-repo")).unwrap();
        repo(root, "org/no-remote", "");
        repo(root, "org/bad name", "git@github.com:org/bad.git");
        repo(root, "bad org/repo", "git@github.com:org/repo.git");
        // A plain file at either level is noise, not a project.
        write(root.join("org/notes.txt"), "hello\n");
        write(root.join(".DS_Store"), "junk\n");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("org"), root.join("linked-org")).unwrap();
            std::os::unix::fs::symlink(root.join("org/kept"), root.join("org/linked-repo"))
                .unwrap();
        }

        let report = scan(root).unwrap();
        assert_eq!(ids(&report), ["org/kept"]);

        let mut found = skips(&report);
        found.sort_unstable();
        let mut expected = vec![
            ("org/bad name", "invalid_id"),
            ("org/no-remote", "no_origin"),
            ("org/not-a-repo", "not_a_repository"),
            ("bad org/repo", "invalid_id"),
        ];
        #[cfg(unix)]
        expected.extend([("linked-org", "symlink"), ("org/linked-repo", "symlink")]);
        expected.sort_unstable();
        assert_eq!(found, expected);
    }

    /// A directory named `.git` with a remote in it is not a clone. The minimum
    /// structure is checked on its own terms — a `HEAD` that reads as a ref, an
    /// object store, a `refs` directory — because otherwise anyone who left a
    /// `config` behind mints a product whose `id` names a working copy nobody can
    /// check out, and tasks get filed against it.
    #[test]
    fn a_config_alone_is_not_a_git_repository() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        let origin = "[remote \"origin\"]\n\turl = git@github.com:org/pseudo.git\n";

        // The whole of a hand-made `.git`: an origin and nothing else.
        write(root.join("org/config-only/.git/config"), origin);

        // HEAD, but not one git wrote.
        let garbled = root.join("org/garbled-head");
        git_dir(&garbled.join(".git"));
        write(garbled.join(".git/config"), origin);
        write(garbled.join(".git/HEAD"), "the main branch, honest\n");

        // Every part but the object store, and every part but refs.
        for (id, missing) in [("org/no-objects", "objects"), ("org/no-refs", "refs")] {
            let dir = root.join(id);
            git_dir(&dir.join(".git"));
            write(dir.join(".git/config"), origin);
            fs::remove_dir_all(dir.join(".git").join(missing)).unwrap();
        }

        // A real clone, as the control: the same config, in a real `.git`.
        repo(root, "org/real", "git@github.com:org/real.git");

        let report = scan(root).unwrap();
        assert_eq!(
            ids(&report),
            ["org/real"],
            "only the clone git would recognise is a product"
        );
        let mut found = skips(&report);
        found.sort_unstable();
        assert_eq!(
            found,
            [
                ("org/config-only", "incomplete_repository"),
                ("org/garbled-head", "incomplete_repository"),
                ("org/no-objects", "incomplete_repository"),
                ("org/no-refs", "incomplete_repository"),
            ]
        );
    }

    /// A worktree and a submodule keep `.git` as a *file* pointing elsewhere, and
    /// the config lives in the common directory that file leads to. Reading
    /// `<repo>/.git/config` directly would skip both for having no remote — and
    /// following the pointer is only allowed because git puts those directories
    /// inside the superproject, which is inside the root.
    #[test]
    fn a_gitdir_pointer_inside_the_root_is_followed_to_the_common_directory() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();

        // The superproject, which owns the git storage the other two borrow.
        let host = repo(root, "org/host", "git@github.com:org/host.git");

        // A worktree: the pointer leads to a per-worktree directory that defers
        // to the superproject's `.git` for config and refs.
        let worktree_git = host.join(".git/worktrees/feature");
        write(worktree_git.join("HEAD"), "ref: refs/heads/feature\n");
        write(worktree_git.join("commondir"), "../..\n");
        let feature = root.join("org/feature");
        fs::create_dir_all(feature.join(".github/workflows")).unwrap();
        fs::write(
            feature.join(".git"),
            "gitdir: ../host/.git/worktrees/feature\n",
        )
        .unwrap();

        // A submodule: the pointer leads to a directory holding its own config
        // and refs, under the superproject's `.git/modules`.
        let module = host.join(".git/modules/sub");
        git_dir(&module);
        write(
            module.join("config"),
            "[remote \"origin\"]\n\turl = git@github.com:org/sub.git\n",
        );
        let sub = root.join("org/sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join(".git"), "gitdir: ../host/.git/modules/sub\n").unwrap();
        write(sub.join("README.md"), "# a submodule\n");

        // A pointer that leads nowhere is not a repository.
        let broken = root.join("org/broken");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join(".git"), "gitdir: ../host/.git/worktrees/gone\n").unwrap();

        let report = scan(root).unwrap();
        assert_eq!(ids(&report), ["org/feature", "org/host", "org/sub"]);

        let worktree = &report.products[0];
        assert_eq!(
            worktree.repository, "https://github.com/org/host",
            "a worktree's config is the superproject's"
        );
        assert!(
            worktree.releases,
            "the workflows are read from the worktree's own working copy"
        );

        let submodule = &report.products[2];
        assert_eq!(submodule.repository, "https://github.com/org/sub");
        assert_eq!(submodule.description, "a submodule");
        assert!(
            !submodule.releases,
            "the shared git directory says nothing about who builds the releases"
        );

        assert_eq!(skips(&report), [("org/broken", "not_a_repository")]);
    }

    /// Nothing outside the root is opened, however the tree asks for it. A `.git`
    /// pointer may be absolute, may climb out with `..`, may be a symlink, and a
    /// worktree's `commondir` may name anything — each is a way to make the walk
    /// read a repository the operator never put in the tree, so each is skipped
    /// and counted rather than followed.
    #[test]
    fn a_git_directory_outside_the_root_is_skipped_not_followed() {
        let (_base, root, outside) = root_and_outside();

        // A complete and tempting repository, outside the tree. Nothing here may
        // reach the catalogue.
        let elsewhere = outside.join("elsewhere.git");
        git_dir(&elsewhere);
        write(
            elsewhere.join("config"),
            "[remote \"origin\"]\n\turl = git@github.com:outside/elsewhere.git\n",
        );
        write(elsewhere.join("refs/tags/v1.0.0"), "abc\n");

        // The clone that is really there, so the walk is not empty for the wrong
        // reason.
        let host = repo(&root, "org/host", "git@github.com:org/host.git");

        let absolute = root.join("org/absolute");
        fs::create_dir_all(&absolute).unwrap();
        fs::write(
            absolute.join(".git"),
            format!("gitdir: {}\n", elsewhere.display()),
        )
        .unwrap();

        // `../../..` from `<root>/org/climbing` is the directory holding the
        // root, so this is the same target spelled relatively.
        let climbing = root.join("org/climbing");
        fs::create_dir_all(&climbing).unwrap();
        fs::write(
            climbing.join(".git"),
            "gitdir: ../../../outside/elsewhere.git\n",
        )
        .unwrap();
        assert_eq!(
            fs::canonicalize(climbing.join("../../../outside/elsewhere.git")).unwrap(),
            fs::canonicalize(&elsewhere).unwrap(),
            "the fixture has to actually reach outside, or it proves nothing"
        );

        // A worktree pointer that stays inside, whose `commondir` does not.
        let escaping_git = host.join(".git/worktrees/escaping");
        write(escaping_git.join("HEAD"), "ref: refs/heads/escaping\n");
        write(
            escaping_git.join("commondir"),
            &format!("{}\n", elsewhere.display()),
        );
        let escaping = root.join("org/escaping");
        fs::create_dir_all(&escaping).unwrap();
        fs::write(
            escaping.join(".git"),
            "gitdir: ../host/.git/worktrees/escaping\n",
        )
        .unwrap();

        #[cfg(unix)]
        {
            let linked = root.join("org/linked");
            fs::create_dir_all(&linked).unwrap();
            std::os::unix::fs::symlink(&elsewhere, linked.join(".git")).unwrap();
        }

        let report = scan(&root).unwrap();
        assert_eq!(
            ids(&report),
            ["org/host"],
            "only the repository inside the tree is a product"
        );
        assert!(
            report
                .products
                .iter()
                .all(|product| !product.repository.contains("outside")),
            "no field may come from outside the root: {:?}",
            report.products
        );

        let mut found = skips(&report);
        found.sort_unstable();
        let mut expected = vec![
            ("org/absolute", "outside_root"),
            ("org/climbing", "outside_root"),
            ("org/escaping", "outside_root"),
        ];
        #[cfg(unix)]
        expected.push(("org/linked", "outside_root"));
        expected.sort_unstable();
        assert_eq!(found, expected);
        let refused = expected.len();
        assert_eq!(
            report.skipped_by_reason().get("outside_root"),
            Some(&refused),
            "the boundary refusals are counted for the startup log"
        );
    }

    /// The metadata a repository inside the root may keep is also inside the
    /// root. A `README.md` or a `.github/workflows` that resolves out of the
    /// tree — through a chain of links, as one has to be able to — is treated as
    /// absent, so a directory the operator never placed can neither describe a
    /// product nor switch its release control on.
    #[cfg(unix)]
    #[test]
    fn metadata_linked_out_of_the_root_is_not_read() {
        let (_base, root, outside) = root_and_outside();

        write(outside.join("secret.md"), "# leaked from outside\n");
        write(outside.join("workflows/release.yml"), "on: push\n");
        // One more hop, so the check cannot be a test of the link's own name.
        let hop = outside.join("hop");
        std::os::unix::fs::symlink(&outside, &hop).unwrap();

        let one = repo(&root, "org/one", "git@github.com:org/one.git");
        std::os::unix::fs::symlink(hop.join("secret.md"), one.join("README.md")).unwrap();
        fs::create_dir_all(one.join(".github")).unwrap();
        std::os::unix::fs::symlink(hop.join("workflows"), one.join(".github/workflows")).unwrap();

        let report = scan(&root).unwrap();
        assert_eq!(ids(&report), ["org/one"], "{:?}", report.skipped);
        let product = &report.products[0];
        assert_eq!(
            product.description, "",
            "a README that resolves outside the root is not the product's description"
        );
        assert!(
            !product.releases,
            "a workflow directory outside the root does not turn release control on"
        );

        // The control: the same two facts, kept inside the tree, do count.
        let two = repo(&root, "org/two", "git@github.com:org/two.git");
        write(two.join("README.md"), "# described from inside\n");
        fs::create_dir_all(two.join(".github/workflows")).unwrap();
        let report = scan(&root).unwrap();
        let inside = &report.products[1];
        assert_eq!(inside.description, "described from inside");
        assert!(inside.releases);
    }

    /// A root that is not there is a misconfiguration, not an empty tree. The
    /// difference matters: an empty answer would be read as "every product was
    /// deleted".
    #[test]
    fn a_root_that_cannot_be_read_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("nowhere");
        assert!(
            matches!(scan(&missing), Err(Error::Io(_))),
            "a missing root must be an error"
        );

        // A file where the tree should be is the same misconfiguration.
        let file = root.path().join("projects");
        fs::write(&file, "not a directory\n").unwrap();
        assert!(matches!(scan(&file), Err(Error::Io(_))));
    }

    /// The retired roster variable is refused, and the refusal says where the
    /// catalogue comes from now. Ignoring it would start a server whose
    /// catalogue silently disagrees with the file the operator still maintains.
    #[test]
    fn the_retired_roster_variable_refuses_the_start() {
        let vars = HashMap::from([
            ("APP_PRODUCTS_SEED", "/etc/products.json"),
            ("APP_PROJECTS_DIR", "/home/user/projects"),
        ]);
        let error = source_from_vars(|key| vars.get(key).map(|v| (*v).to_owned()))
            .expect_err("a retired variable must refuse the start");
        let Error::Invalid(message) = &error else {
            panic!("expected a bad-configuration refusal, got {error:?}");
        };
        assert!(
            message.contains("APP_PRODUCTS_SEED") && message.contains("APP_PROJECTS_DIR"),
            "the refusal must name both the retired variable and its replacement: {message}"
        );
    }

    /// With no tree configured the catalogue is left to the API, and with one it
    /// is derived. An empty value is not a configuration.
    #[test]
    fn the_tree_is_configured_by_one_variable_or_not_at_all() {
        let derived = source_from_vars(|key| match key {
            "APP_PROJECTS_DIR" => Some("/home/user/projects".to_owned()),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            derived,
            Catalogue::Derived(PathBuf::from("/home/user/projects"))
        );

        assert_eq!(source_from_vars(|_| None).unwrap(), Catalogue::Curated);
        assert_eq!(
            source_from_vars(|key| match key {
                "APP_PROJECTS_DIR" | "APP_PRODUCTS_SEED" => Some(String::new()),
                _ => None,
            })
            .unwrap(),
            Catalogue::Curated,
            "an empty value asks for nothing"
        );
    }
}
