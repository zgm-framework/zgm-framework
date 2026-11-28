//! The order of a list.

/// The order of a list, by the moment each row was created.
///
/// The module sorts by `id`: a UUID v7 starts with the moment it was made, and no two rows
/// share one, so a list that goes on over several pages never skips or repeats a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Order {
    /// The newest first. A list without an order goes this way.
    #[default]
    NewestFirst,
    /// The oldest first.
    OldestFirst,
}
