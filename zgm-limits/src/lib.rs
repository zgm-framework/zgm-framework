//! `zgm-limits`: limits on attempts, counted in PostgreSQL. The new version of `zgm-limits`.
//!
//! The product lists its limits in an enum and implements [`Limit`] for it; [`Limits`] counts
//! attempts and answers with an [`Answer`].

mod answer;
mod attempts;
mod audit;
mod blocked;
mod clock;
mod error;
mod jobs;
mod keyed_hash;
mod limits;
mod migrate;
mod repository;
mod rule;
mod tenant_rules;

pub use answer::Answer;
pub use attempts::Attempts;
pub use audit::{Audit, AuditError, EventKey, LiftedBy, LimitsEvent};
pub use blocked::{Blocked, BlockedCursor, BlockedPage};
pub use clock::{Clock, SystemClock};
pub use error::LimitsError;
pub use jobs::{Job, JobError};
pub use keyed_hash::{KeyedHash, KeyedHashError, MAX_HASH_BYTES};
pub use limits::Limits;
pub use migrate::migrate;
pub use rule::{Bounds, Count, Limit, MAX_DISTINCT_ELEMENTS, Period, Rule};
pub use tenant_rules::{DefaultRule, TenantRule};
