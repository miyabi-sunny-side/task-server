use crate::error::Error;
use crate::frontmatter::{Document, get_str};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Draft,
    Ready,
    Running,
    AwaitingUser,
    Done,
    ReleaseRequested,
    Released,
    ReleaseFailed,
    Blocked,
    Cancelled,
    Dropped,
}

impl Status {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::AwaitingUser => "awaiting_user",
            Self::Done => "done",
            Self::ReleaseRequested => "release_requested",
            Self::Released => "released",
            Self::ReleaseFailed => "release_failed",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
            Self::Dropped => "dropped",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, Error> {
        match raw {
            "draft" => Ok(Self::Draft),
            "ready" => Ok(Self::Ready),
            "running" => Ok(Self::Running),
            "awaiting_user" => Ok(Self::AwaitingUser),
            "done" => Ok(Self::Done),
            "release_requested" => Ok(Self::ReleaseRequested),
            "released" => Ok(Self::Released),
            "release_failed" => Ok(Self::ReleaseFailed),
            "blocked" => Ok(Self::Blocked),
            "cancelled" => Ok(Self::Cancelled),
            "dropped" => Ok(Self::Dropped),
            other => Err(Error::Invalid(format!("invalid status: {other}"))),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TransitionContext {
    pub target_space: Option<String>,
    pub product_id: Option<String>,
}

impl TransitionContext {
    #[must_use]
    pub fn from_document(doc: &Document) -> Self {
        Self {
            target_space: get_str(&doc.frontmatter, "target_space"),
            product_id: get_str(&doc.frontmatter, "product_id"),
        }
    }

    #[must_use]
    pub fn effective_product_id(&self) -> Option<&str> {
        match (self.target_space.as_deref(), self.product_id.as_deref()) {
            (Some(space), Some(product)) if space == product => Some(space),
            (Some(space), None) => Some(space),
            (None, Some(product)) => Some(product),
            (Some(_), Some(_)) | (None, None) => None,
        }
    }

    #[must_use]
    pub fn is_self_service(&self) -> bool {
        self.effective_product_id() == Some("household/tasks")
    }
}

/// Whether `from → to` is allowed for this task.
#[must_use]
pub fn can_transition(from: Status, to: Status, ctx: &TransitionContext) -> bool {
    if matches!(to, Status::Blocked | Status::Cancelled | Status::Dropped) {
        return !matches!(from, Status::Released | Status::Dropped);
    }
    match (from, to) {
        (Status::Draft, Status::Ready)
        | (Status::Ready, Status::Running)
        | (Status::Running, Status::AwaitingUser | Status::Running)
        | (Status::AwaitingUser, Status::Done | Status::Ready | Status::ReleaseRequested)
        | (Status::Done, Status::ReleaseRequested)
        | (Status::ReleaseRequested, Status::Released | Status::ReleaseFailed) => true,
        (Status::Ready, Status::AwaitingUser) => ctx.is_self_service(),
        _ => false,
    }
}

/// Required-field and form checks shared with tasks `bin/check`.
pub fn validate_task(doc: &Document) -> Result<(), Error> {
    let kind = get_str(&doc.frontmatter, "type").unwrap_or_default();
    if kind != "Task" {
        return Err(Error::Invalid("frontmatter type must be Task".into()));
    }
    let status_raw = get_str(&doc.frontmatter, "status")
        .ok_or_else(|| Error::Invalid("missing status".into()))?;
    let status = Status::parse(&status_raw)?;
    let area = get_str(&doc.frontmatter, "area").unwrap_or_default();
    if area != "development" && area != "household" {
        return Err(Error::Invalid(format!(
            "invalid area '{area}' (development|household)"
        )));
    }
    if let Some(due) = get_str(&doc.frontmatter, "due")
        && !due_ok(&due)
    {
        return Err(Error::Invalid(format!("invalid due '{due}' (YYYY-MM-DD)")));
    }
    if status == Status::Ready && get_str(&doc.frontmatter, "next_action").is_none() {
        return Err(Error::Invalid("ready task without next_action".into()));
    }
    let target_space = get_str(&doc.frontmatter, "target_space");
    let product_id = get_str(&doc.frontmatter, "product_id");
    if let Some(ref space) = target_space {
        check_product_id("target_space", space)?;
    }
    if let Some(ref alias) = product_id {
        check_product_id("product_id", alias)?;
    }
    if let (Some(space), Some(alias)) = (&target_space, &product_id)
        && space != alias
    {
        return Err(Error::Invalid(
            "conflicting target_space and product_id".into(),
        ));
    }
    if matches!(
        status,
        Status::Ready | Status::Running | Status::AwaitingUser
    ) && area == "development"
        && target_space.is_none()
        && product_id.is_none()
    {
        return Err(Error::Invalid(format!(
            "{status} development task without target_space or product_id",
            status = status.as_str()
        )));
    }
    if status == Status::Running {
        require_field(doc, "claim_id")?;
        require_field(doc, "worker")?;
        check_datetime_field(doc, "claimed_at")?;
        check_datetime_field(doc, "claim_expires_at")?;
    }
    if status == Status::AwaitingUser {
        require_field(doc, "commit_sha")?;
        require_field(doc, "verification")?;
    }
    if matches!(
        status,
        Status::ReleaseRequested | Status::Released | Status::ReleaseFailed
    ) {
        let repo = require_field(doc, "release_repo")?;
        check_product_id("release_repo", &repo)?;
        let sha = require_field(doc, "release_sha")?;
        if !git_sha_ok(&sha) {
            return Err(Error::Invalid(format!(
                "invalid release_sha '{sha}' (7-40 hex)"
            )));
        }
        let bump = require_field(doc, "bump")?;
        if !matches!(bump.as_str(), "patch" | "minor" | "major") {
            return Err(Error::Invalid(format!(
                "{status} task without valid bump (patch|minor|major)",
                status = status.as_str()
            )));
        }
    }
    if status == Status::Released {
        require_field(doc, "release_tag")?;
    }
    if status == Status::ReleaseFailed {
        require_field(doc, "failure")?;
    }
    Ok(())
}

fn require_field(doc: &Document, name: &str) -> Result<String, Error> {
    get_str(&doc.frontmatter, name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Error::Invalid(format!("missing {name}")))
}

pub(crate) fn check_product_id(name: &str, value: &str) -> Result<(), Error> {
    let invalid = || Error::Invalid(format!("invalid {name} '{value}' (org/repo, not a path)"));
    if value.contains('\\') || value.contains("..") {
        return Err(invalid());
    }
    let mut parts = value.split('/');
    let (Some(org), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(invalid());
    };
    if !segment_ok(org) || !segment_ok(repo) {
        return Err(invalid());
    }
    Ok(())
}

fn segment_ok(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

fn due_ok(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..].iter().all(u8::is_ascii_digit)
}

fn git_sha_ok(raw: &str) -> bool {
    let len = raw.len();
    (7..=40).contains(&len) && raw.bytes().all(|b| b.is_ascii_hexdigit())
}

fn check_datetime_field(doc: &Document, name: &str) -> Result<(), Error> {
    let raw = require_field(doc, name)?;
    if datetime_ok(&raw) {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "invalid {name} '{raw}' (YYYY-MM-DDTHH:MM:SSZ or YYYY-MM-DDTHH:MM:SS±HH:MM)"
        )))
    }
}

fn datetime_ok(raw: &str) -> bool {
    let (head, tail) = if let Some(stripped) = raw.strip_suffix('Z') {
        (stripped, "Z")
    } else if raw.len() >= 6 {
        let split = raw.len() - 6;
        (&raw[..split], &raw[split..])
    } else {
        return false;
    };
    if head.len() != 19 || head.as_bytes()[10] != b'T' {
        return false;
    }
    let date = &head[..10];
    let time = &head[11..];
    due_ok(date)
        && time.len() == 8
        && time.as_bytes()[2] == b':'
        && time.as_bytes()[5] == b':'
        && time.as_bytes()[0..2].iter().all(u8::is_ascii_digit)
        && time.as_bytes()[3..5].iter().all(u8::is_ascii_digit)
        && time.as_bytes()[6..8].iter().all(u8::is_ascii_digit)
        && (tail == "Z"
            || (tail.len() == 6
                && (tail.starts_with('+') || tail.starts_with('-'))
                && tail.as_bytes()[3] == b':'
                && tail.as_bytes()[1..3].iter().all(u8::is_ascii_digit)
                && tail.as_bytes()[4..6].iter().all(u8::is_ascii_digit)))
}
