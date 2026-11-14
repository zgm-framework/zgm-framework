//! What can go wrong in the module.

use crate::audit::AuditError;
use crate::keyed_hash::KeyedHashError;

/// An error of the module. A refused attempt is not an error: it is an
/// [`Answer`](crate::Answer).
#[derive(Debug, thiserror::Error)]
pub enum LimitsError {
    /// The database failed.
    #[error("the database failed: {0}")]
    Database(#[from] sqlx::Error),
    /// A migration of the module's schema failed.
    #[error("the migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    /// An attempt got no answer although it was asked several times: other attempts and
    /// resets kept changing its counter in between. The attempt may or may not be counted;
    /// treat it as refused.
    #[error("no answer for the attempt after {tries} tries")]
    NoAnswer {
        /// How many times the attempt was asked.
        tries: u32,
    },
    /// The `KeyedHash` of the builder failed, e.g. it could not fetch its key.
    #[error("the keyed hash failed: {0}")]
    KeyedHash(#[from] KeyedHashError),
    /// The `Audit` of the builder failed, or panicked, while it was taking the event of a
    /// change. The change was rolled back.
    #[error("the audit failed: {0}")]
    Audit(#[from] AuditError),
    /// The `KeyedHash` of the builder returned no hash, or an empty one, for a key with a
    /// hidden part.
    #[error("the keyed hash returned no hash or an empty hash")]
    KeyedHashGaveNoHash,
    /// The `KeyedHash` of the builder returned a hash longer than
    /// [`MAX_HASH_BYTES`](crate::MAX_HASH_BYTES), e.g. the data itself instead of a hash.
    #[error("the keyed hash returned a hash longer than 128 bytes")]
    KeyedHashGaveLongHash,
    /// An attempt was given the nil UUID (all zeros) as its account. That value means "no
    /// account"; for the attempts of the whole tenant, give no account at all.
    #[error("the nil UUID is not an account; give no `subject_id` for the whole tenant")]
    NilSubject,
    /// An attempt was given both `hidden` and `hidden_exact`; a key has one hidden part.
    #[error("both `hidden` and `hidden_exact` were given; a key has one hidden part")]
    HiddenAndHiddenExact,
    /// An attempt names a limit that was not on the list given to the builder, so its rule was
    /// never read when the module was built.
    #[error("the limit `{limit}` was not given to the builder")]
    UnknownLimit {
        /// The name of the limit.
        limit: &'static str,
    },
    /// A tenant's rule was given for a limit that has no bounds in the code: such a limit
    /// cannot be changed by a tenant.
    #[error("the rule of the limit `{limit}` cannot be changed by a tenant")]
    RuleCannotBeChanged {
        /// The name of the limit.
        limit: &'static str,
    },
    /// A tenant's rule has a number outside what the limit allows.
    #[error(
        "`{field}` of {value} is outside what the limit `{limit}` allows, {lowest} to {highest}"
    )]
    RuleOutOfBounds {
        /// The name of the limit.
        limit: &'static str,
        /// Which number: `at_most` or `per_seconds`.
        field: &'static str,
        /// The number that was given.
        value: u32,
        /// The lowest the limit allows.
        lowest: u32,
        /// The highest the limit allows.
        highest: u32,
    },
    /// `hit` was called on a limit that counts distinct elements, or `hit_distinct` on one
    /// that counts attempts. Which of the two a limit takes is in its rule in the code.
    #[error("the limit `{limit}` is not counted with `{called}`: call `{call_instead}`")]
    WrongWayOfCounting {
        /// The name of the limit.
        limit: &'static str,
        /// The method that was called.
        called: &'static str,
        /// The method the rule of the limit takes.
        call_instead: &'static str,
    },
    /// A tenant's rule has another way of counting than the default of the limit. Whether a
    /// limit counts attempts or distinct elements is decided in the code, by the call.
    #[error("the rule for the limit `{limit}` must count the same way as its default")]
    RuleOfAnotherKind {
        /// The name of the limit.
        limit: &'static str,
    },
    /// The limit counts with a bucket, and counting a bucket is not implemented yet.
    #[error("counting a bucket is not implemented yet")]
    BucketNotImplemented,
}
