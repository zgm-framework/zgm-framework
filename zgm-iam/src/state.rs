//! The state of a tenant or a user.

/// The state of a tenant or of a user.
///
/// This type holds the list of states. The database keeps the word of each, `active`,
/// `suspended` or `pending_deletion`, as plain text and does not check it, because only the
/// module's code writes it. `str::parse` reads a word back, and `<&str>::from` gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumString, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum State {
    /// Works as usual. Every new tenant and user starts here.
    Active,
    /// Blocked: nothing works on it until it is unblocked.
    Suspended,
    /// Waits for deletion: nothing works on it, it can still be restored, and after its date
    /// of deletion it is gone.
    PendingDeletion,
}
