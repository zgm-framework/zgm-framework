//! A user, as the module gives it back.

use jiff::Timestamp;
use uuid::Uuid;

use crate::state::State;

/// A user of a tenant. The module keeps nothing that says who the user is: the application
/// keeps the name and the e-mail, and the login module what the user signs in with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// The identifier the database gave the user, a UUID v7.
    pub id: Uuid,
    /// The tenant the user belongs to.
    pub tenant_id: Uuid,
    /// Whether the user works, is blocked or waits for deletion.
    pub state: State,
    /// When the user will be deleted, while it is [`State::PendingDeletion`]; `None` in the
    /// other states.
    pub deletes_at: Option<Timestamp>,
    /// When the user was created.
    pub created_at: Timestamp,
    /// When the user last changed; the same as `created_at` for a user that never did.
    pub updated_at: Timestamp,
}

/// One page of the list of the users of a tenant, from
/// [`TenantScope::list_users`](crate::TenantScope::list_users).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsersPage {
    /// The users of the page, at most 100, in the order the list was asked for.
    pub items: Vec<User>,
    /// Where the next page starts: the `id` of the last user of this page, to give to `after`.
    /// `None` when this page is the last one.
    pub next_cursor: Option<Uuid>,
}
