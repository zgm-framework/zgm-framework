//! Where the module takes the current time from.

use std::fmt;

use chrono::{DateTime, Utc};

/// The source of the current time. The module asks it once per attempt.
pub trait Clock: fmt::Debug + Send + Sync + 'static {
    /// The current time in UTC.
    fn now(&self) -> DateTime<Utc>;
}

/// The operating system's clock, in UTC. The builder uses it unless a test puts in another one.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}
