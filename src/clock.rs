use std::sync::{Arc, Mutex};

use time::macros::format_description;
use time::{OffsetDateTime, UtcOffset};

pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

#[derive(Clone)]
pub struct SharedClock {
    now: Arc<Mutex<OffsetDateTime>>,
}

impl SharedClock {
    #[must_use]
    pub fn at(now: OffsetDateTime) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    pub fn set(&self, now: OffsetDateTime) {
        *self.now.lock().expect("clock mutex") = now;
    }

    pub fn advance_secs(&self, secs: i64) {
        let mut guard = self.now.lock().expect("clock mutex");
        *guard += time::Duration::seconds(secs);
    }
}

impl Clock for SharedClock {
    fn now(&self) -> OffsetDateTime {
        *self.now.lock().expect("clock mutex")
    }
}

/// Render a datetime as the stored `YYYY-MM-DDTHH:MM:SSZ` form.
#[must_use]
pub fn format_z(dt: OffsetDateTime) -> String {
    const FORMAT: &[time::format_description::FormatItem<'static>] =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");
    dt.to_offset(UtcOffset::UTC)
        .format(FORMAT)
        .expect("datetime format")
}
