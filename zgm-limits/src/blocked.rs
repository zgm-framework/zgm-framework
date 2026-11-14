//! The list of blocked keys.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// One blocked key: in this limit no attempt of it passes until `blocked_until`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocked {
    /// The name of the row, for [`Limits::unblock`](crate::Limits::unblock).
    pub id: i64,
    /// The name of the limit. A `String` and not a name from the code, because it is read from
    /// the database: a key may still be blocked in a limit the code no longer has.
    pub limit_name: String,
    /// The tenant.
    pub tenant_id: Uuid,
    /// The account the counter is about; [`Uuid::nil`] when the key has none.
    pub subject_id: Uuid,
    /// The hash of the hidden part; empty when the key has none.
    pub key_hash: Vec<u8>,
    /// Whether the counter is of the whole tenant: the key has neither an account nor a hidden
    /// part, so the block stops everybody in the tenant.
    pub whole_tenant: bool,
    /// The moment the key got blocked.
    pub blocked_at: DateTime<Utc>,
    /// The moment from which an attempt may pass again.
    pub blocked_until: DateTime<Utc>,
    /// Whether the block is already over.
    pub expired: bool,
}

/// One page of the list of blocked keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedPage {
    /// The blocked keys of the page, the newest block first.
    pub items: Vec<Blocked>,
    /// Where the next page starts; `None` when this page is the last one.
    pub next_cursor: Option<BlockedCursor>,
}

/// The place in the list of blocked keys a page ended at: its last row. Both fields are public
/// so that a panel can carry them in a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockedCursor {
    /// `blocked_at` of the last row of the page.
    pub blocked_at: DateTime<Utc>,
    /// `id` of the last row of the page.
    pub id: i64,
}
