//! A tenant, as the module gives it back.

use jiff::Timestamp;
use uuid::Uuid;

use crate::state::State;

/// A tenant of the application: the root under which everything else of the module lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tenant {
    /// The identifier the database gave the tenant, a UUID v7.
    pub id: Uuid,
    /// The name, readable. The table holds only what the application's
    /// [`Cipher`](crate::Cipher) made of it.
    pub name: String,
    /// Whether the tenant works, is blocked or waits for deletion.
    pub state: State,
    /// When the tenant will be deleted, while it is [`State::PendingDeletion`]; `None` in the
    /// other states.
    pub deletes_at: Option<Timestamp>,
    /// When the tenant was created.
    pub created_at: Timestamp,
    /// When the tenant last changed; the same as `created_at` for a tenant that never did.
    pub updated_at: Timestamp,
}
