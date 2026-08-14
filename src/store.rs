use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

use crate::actions::ActionTable;
use crate::error::Error;
use crate::frontmatter::{Document, get_str, join_document, set_str, split_document};
use crate::notify::flush_pending;
use crate::outbox::{NotificationIntent, enqueue};
use crate::state::AppState;
use crate::status::{Status, TransitionContext, can_transition, validate_task};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimLease {
    pub task_id: String,
    pub claim_id: String,
    pub claimed_at: String,
    pub claim_expires_at: String,
    pub status: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportRequest {
    pub claim_id: String,
    pub commit_sha: String,
    pub verification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReportOutcome {
    pub status: String,
    pub task_id: String,
    pub commit_sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskCard {
    pub id: String,
    pub title: String,
    pub status: String,
    pub body: String,
    pub verification: Option<String>,
    pub commit_sha: Option<String>,
    pub available_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskSummary {
    pub id: String,
    pub title: String,
    pub status: String,
}

struct Loaded {
    id: String,
    path: PathBuf,
    doc: Document,
}

pub fn claim(state: &AppState, worker: &str) -> Result<Option<ClaimLease>, Error> {
    if worker.trim().is_empty() {
        return Err(Error::Invalid("worker is required".into()));
    }
    with_lock(state, || {
        assert_clean(&state.tasks_git_dir)?;
        let now = state.clock.now();
        let mut tasks = load_all(state)?;
        let chosen = tasks.iter().position(|task| is_claimable(&task.doc, now));
        let Some(index) = chosen else {
            return Ok(None);
        };
        let ttl = i64::try_from(state.claim_ttl_secs).unwrap_or(i64::MAX);
        let claimed_at = format_z(now);
        let claim_expires_at = format_z(now + time::Duration::seconds(ttl));
        let claim_id = uuid::Uuid::new_v4().to_string();
        set_str(&mut tasks[index].doc.frontmatter, "status", "running");
        set_str(&mut tasks[index].doc.frontmatter, "claim_id", &claim_id);
        set_str(&mut tasks[index].doc.frontmatter, "claimed_at", &claimed_at);
        set_str(&mut tasks[index].doc.frontmatter, "worker", worker);
        set_str(
            &mut tasks[index].doc.frontmatter,
            "claim_expires_at",
            &claim_expires_at,
        );
        let task_id = tasks[index].id.clone();
        let title = title_of(&tasks[index]);
        let body = String::from_utf8_lossy(&tasks[index].doc.body).into_owned();
        validate_task(&tasks[index].doc)?;
        write_and_commit(
            state,
            &tasks[index],
            &format!("claim {task_id} for {worker}"),
        )?;
        Ok(Some(ClaimLease {
            task_id,
            claim_id,
            claimed_at,
            claim_expires_at,
            status: "running".into(),
            title,
            body,
        }))
    })
}

pub fn report(state: &AppState, request: &ReportRequest) -> Result<ReportOutcome, Error> {
    if request.claim_id.trim().is_empty()
        || request.commit_sha.trim().is_empty()
        || request.verification.trim().is_empty()
    {
        return Err(Error::Invalid(
            "claim_id, commit_sha, and verification are required".into(),
        ));
    }
    with_lock(state, || {
        let mut tasks = load_all(state)?;
        let index = tasks.iter().position(|task| {
            get_str(&task.doc.frontmatter, "claim_id").as_deref() == Some(request.claim_id.as_str())
        });
        let Some(index) = index else {
            return Err(Error::ClaimMismatch);
        };
        let status = status_of(&tasks[index].doc)
            .ok_or_else(|| Error::Invalid(format!("task {} has no status", tasks[index].id)))?;
        if status == Status::AwaitingUser {
            let same_sha = get_str(&tasks[index].doc.frontmatter, "commit_sha").as_deref()
                == Some(request.commit_sha.as_str());
            if same_sha {
                return Ok(ReportOutcome {
                    status: "awaiting_user".into(),
                    task_id: tasks[index].id.clone(),
                    commit_sha: request.commit_sha.clone(),
                });
            }
            return Err(Error::Invalid(
                "report already applied with a different commit_sha".into(),
            ));
        }
        if status != Status::Running {
            return Err(Error::Invalid(format!(
                "cannot report from {}",
                status.as_str()
            )));
        }
        assert_clean(&state.tasks_git_dir)?;
        set_str(&mut tasks[index].doc.frontmatter, "status", "awaiting_user");
        set_str(
            &mut tasks[index].doc.frontmatter,
            "commit_sha",
            &request.commit_sha,
        );
        set_str(
            &mut tasks[index].doc.frontmatter,
            "verification",
            &request.verification,
        );
        let task_id = tasks[index].id.clone();
        validate_task(&tasks[index].doc)?;
        write_and_commit(
            state,
            &tasks[index],
            &format!("report {task_id} awaiting_user"),
        )?;
        let intent = NotificationIntent {
            task_id: task_id.clone(),
            kind: "awaiting_user".into(),
            commit_sha: request.commit_sha.clone(),
            claim_id: request.claim_id.clone(),
            created_at: format_z(state.clock.now()),
            state: "pending".into(),
        };
        enqueue(&state.outbox_dir, &intent)?;
        let _ = flush_pending(&state.outbox_dir, state.notifier.as_ref());
        Ok(ReportOutcome {
            status: "awaiting_user".into(),
            task_id,
            commit_sha: request.commit_sha.clone(),
        })
    })
}

pub fn apply_human_action(
    state: &AppState,
    task_id: &str,
    action: &str,
    bump: Option<&str>,
) -> Result<TaskCard, Error> {
    with_lock(state, || {
        assert_clean(&state.tasks_git_dir)?;
        let mut tasks = load_all(state)?;
        let index = tasks
            .iter()
            .position(|task| task.id == task_id)
            .ok_or(Error::NotFound)?;
        let status = status_of(&tasks[index].doc).ok_or(Error::Invalid("missing status".into()))?;
        let ctx = TransitionContext::from_document(&tasks[index].doc);
        let effect = state.action_table.translate(status, action, bump)?;
        if !can_transition(status, effect.to, &ctx) {
            return Err(Error::ActionNotAllowed);
        }
        set_str(
            &mut tasks[index].doc.frontmatter,
            "status",
            effect.to.as_str(),
        );
        if let Some(next) = &effect.next_action {
            set_str(&mut tasks[index].doc.frontmatter, "next_action", next);
        }
        if effect.set_release {
            let repo = ctx
                .effective_product_id()
                .ok_or_else(|| {
                    Error::Invalid("release_repo requires target_space or product_id".into())
                })?
                .to_owned();
            let sha = get_str(&tasks[index].doc.frontmatter, "commit_sha")
                .ok_or_else(|| Error::Invalid("release_sha requires commit_sha".into()))?;
            set_str(&mut tasks[index].doc.frontmatter, "release_repo", &repo);
            set_str(&mut tasks[index].doc.frontmatter, "release_sha", &sha);
            if let Some(kind) = &effect.bump {
                set_str(&mut tasks[index].doc.frontmatter, "bump", kind);
            }
        }
        validate_task(&tasks[index].doc)?;
        write_and_commit(state, &tasks[index], &format!("action {task_id} {action}"))?;
        Ok(card_from(&tasks[index], &state.action_table))
    })
}

pub fn self_service_awaiting_user(
    state: &AppState,
    task_id: &str,
    commit_sha: &str,
    verification: &str,
) -> Result<TaskCard, Error> {
    if commit_sha.trim().is_empty() || verification.trim().is_empty() {
        return Err(Error::Invalid(
            "commit_sha and verification are required".into(),
        ));
    }
    with_lock(state, || {
        assert_clean(&state.tasks_git_dir)?;
        let mut tasks = load_all(state)?;
        let index = tasks
            .iter()
            .position(|task| task.id == task_id)
            .ok_or(Error::NotFound)?;
        let status = status_of(&tasks[index].doc).ok_or(Error::Invalid("missing status".into()))?;
        let ctx = TransitionContext::from_document(&tasks[index].doc);
        if !can_transition(status, Status::AwaitingUser, &ctx) {
            return Err(Error::ActionNotAllowed);
        }
        set_str(
            &mut tasks[index].doc.frontmatter,
            "status",
            Status::AwaitingUser.as_str(),
        );
        set_str(&mut tasks[index].doc.frontmatter, "commit_sha", commit_sha);
        set_str(
            &mut tasks[index].doc.frontmatter,
            "verification",
            verification,
        );
        validate_task(&tasks[index].doc)?;
        write_and_commit(
            state,
            &tasks[index],
            &format!("self-service {task_id} awaiting_user"),
        )?;
        Ok(card_from(&tasks[index], &state.action_table))
    })
}

pub fn list_tasks(state: &AppState) -> Result<Vec<TaskSummary>, Error> {
    let tasks = load_all(state)?;
    Ok(tasks
        .into_iter()
        .map(|task| TaskSummary {
            id: task.id.clone(),
            title: title_of(&task),
            status: get_str(&task.doc.frontmatter, "status").unwrap_or_default(),
        })
        .collect())
}

pub fn get_task(state: &AppState, id: &str) -> Result<TaskCard, Error> {
    let tasks = load_all(state)?;
    let task = tasks
        .into_iter()
        .find(|task| task.id == id)
        .ok_or(Error::NotFound)?;
    Ok(card_from(&task, &state.action_table))
}

fn card_from(task: &Loaded, table: &ActionTable) -> TaskCard {
    let status_raw = get_str(&task.doc.frontmatter, "status").unwrap_or_default();
    let status = Status::parse(&status_raw).unwrap_or(Status::Draft);
    TaskCard {
        id: task.id.clone(),
        title: title_of(task),
        status: status_raw,
        body: String::from_utf8_lossy(&task.doc.body).into_owned(),
        verification: get_str(&task.doc.frontmatter, "verification"),
        commit_sha: get_str(&task.doc.frontmatter, "commit_sha"),
        available_actions: table.available_actions(status),
    }
}

fn title_of(task: &Loaded) -> String {
    get_str(&task.doc.frontmatter, "title").unwrap_or_else(|| task.id.clone())
}

fn status_of(doc: &Document) -> Option<Status> {
    Status::parse(&get_str(&doc.frontmatter, "status")?).ok()
}

fn is_claimable(doc: &Document, now: OffsetDateTime) -> bool {
    let Some(status) = status_of(doc) else {
        return false;
    };
    let ctx = TransitionContext::from_document(doc);
    if ctx.is_self_service() || get_str(&doc.frontmatter, "area").as_deref() == Some("household") {
        return false;
    }
    match status {
        Status::Ready => validate_task(doc).is_ok(),
        Status::Running => get_str(&doc.frontmatter, "claim_expires_at")
            .and_then(|raw| parse_dt(&raw).ok())
            .is_some_and(|expires| expires <= now),
        _ => false,
    }
}

fn load_all(state: &AppState) -> Result<Vec<Loaded>, Error> {
    let dir = state.tasks_git_dir.join("projects/queue/tasks");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<_> = fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("md"))
        .collect();
    paths.sort();
    let mut tasks = Vec::new();
    for path in paths {
        let bytes = fs::read(&path)?;
        let doc = split_document(&bytes)?;
        let id = path
            .file_stem()
            .ok_or_else(|| Error::Invalid("task path has no stem".into()))?
            .to_string_lossy()
            .into_owned();
        tasks.push(Loaded { id, path, doc });
    }
    Ok(tasks)
}

fn write_and_commit(state: &AppState, loaded: &Loaded, message: &str) -> Result<String, Error> {
    let original = fs::read(&loaded.path).unwrap_or_default();
    let bytes = join_document(&loaded.doc)?;
    let tmp = loaded.path.with_extension("md.tmp");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, &loaded.path)?;
    let rel = format!("projects/queue/tasks/{}.md", loaded.id);
    let committed = (|| {
        git(&state.tasks_git_dir, &["add", "--", &rel])?;
        git(&state.tasks_git_dir, &["commit", "-m", message])?;
        git(&state.tasks_git_dir, &["rev-parse", "HEAD"])
    })();
    match committed {
        Ok(sha) => Ok(sha.trim().to_owned()),
        Err(err) => {
            let _ = fs::write(&loaded.path, original);
            let _ = git(&state.tasks_git_dir, &["checkout", "--", &rel]);
            Err(err)
        }
    }
}

fn with_lock<T>(state: &AppState, func: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
    fs::create_dir_all(&state.tasks_git_dir)?;
    let lock_path = state.tasks_git_dir.join(".task-server.lock");
    let file = fs::File::create(lock_path)?;
    file.lock_exclusive()
        .map_err(|err| Error::Io(err.to_string()))?;
    func()
}

fn assert_clean(dir: &Path) -> Result<(), Error> {
    let status = git(dir, &["status", "--porcelain"])?;
    for line in status.lines() {
        let path = line.get(3..).unwrap_or("").trim();
        if path.is_empty()
            || path == ".outbox"
            || path.starts_with(".outbox/")
            || path == ".task-server.lock"
        {
            continue;
        }
        return Err(Error::DirtyWorktree);
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<String, Error> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|err| Error::Git(err.to_string()))?;
    if !output.status.success() {
        return Err(Error::Git(format!(
            "{}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn format_z(dt: OffsetDateTime) -> String {
    const FORMAT: &[time::format_description::FormatItem<'static>] =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    dt.to_offset(UtcOffset::UTC)
        .format(FORMAT)
        .expect("datetime format")
}

fn parse_dt(raw: &str) -> Result<OffsetDateTime, Error> {
    const NAIVE: &[time::format_description::FormatItem<'static>] =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]");
    if let Some(stripped) = raw.strip_suffix('Z') {
        let primitive = PrimitiveDateTime::parse(stripped, NAIVE)
            .map_err(|err| Error::Invalid(format!("invalid datetime: {err}")))?;
        return Ok(primitive.assume_utc());
    }
    OffsetDateTime::parse(raw, &Rfc3339)
        .or_else(|_| {
            const OFFSET: &[time::format_description::FormatItem<'static>] = format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second][offset_hour sign:mandatory]:[offset_minute]"
            );
            OffsetDateTime::parse(raw, OFFSET)
        })
        .map_err(|err| Error::Invalid(format!("invalid datetime: {err}")))
}
