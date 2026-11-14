//! What the module says about one attempt.

use chrono::{DateTime, Utc};

/// The answer to one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use = "an attempt that was refused must not go on: check whether the answer is `Allowed`"]
pub enum Answer {
    /// The attempt passed and was counted.
    Allowed,
    /// The attempt did not pass and nothing was counted.
    Refused {
        /// The moment from which an attempt may pass again.
        until: DateTime<Utc>,
    },
}
