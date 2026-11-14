//! What the module reports about its own work.

use std::error::Error;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::rule::Rule;

/// Where the module reports what it did. The application gives an implementation to the
/// builder; it may print the events, count them, or write them to its audit log.
///
/// The module has no logging of its own: every place where a line of a log would be reports
/// an event here instead.
///
/// There are two ways an event arrives, and which one is decided by what the event is about:
///
/// - **A change somebody made** ([`LimitsEvent::BlockLifted`], [`LimitsEvent::RuleChanged`])
///   comes to [`record_in_transaction`](Self::record_in_transaction), inside the transaction
///   that makes the change and before its commit. Written with the connection given there,
///   the event is committed together with the change, or not at all.
/// - **What changed nothing an event could be written together with**
///   ([`LimitsEvent::AttemptRefused`], [`LimitsEvent::KeyBlocked`],
///   [`LimitsEvent::ExpiredDeleted`]) comes to
///   [`record_after_the_fact`](Self::record_after_the_fact), like a line of a log. An attempt
///   is one statement with no transaction around it, so that a flood of attempts costs the
///   database one round trip each.
#[async_trait]
pub trait Audit: Send + Sync + 'static {
    /// Takes the event of a change, inside the transaction of that change.
    ///
    /// What is written with `transaction` is committed or rolled back together with the
    /// change. An error rolls the change back and is the error of the operation
    /// ([`LimitsError::Audit`](crate::LimitsError)), and so does a panic. The call is awaited
    /// on the path of the request, so it must be quick.
    async fn record_in_transaction(
        &self,
        transaction: &mut PgConnection,
        event: LimitsEvent,
    ) -> Result<(), AuditError>;

    /// Takes an event after the fact. It cannot fail, and a panic in it is caught and
    /// dropped. An application that stops between the attempt and this call loses the event;
    /// a block is still in the table, and on the list of blocked keys.
    ///
    /// It is called on the path of the request and of the cleanup, after the connection went
    /// back to the pool, and it must return at once; an implementation that writes to a
    /// database hands the event to a queue or a task of its own.
    fn record_after_the_fact(&self, event: LimitsEvent);
}

/// Why an [`Audit`] could not take an event. It wraps the implementation's own error.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct AuditError(Box<dyn Error + Send + Sync>);

impl AuditError {
    /// Wraps the error of an implementation.
    pub fn new(error: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self(error.into())
    }
}

/// One thing the module did. No event holds the hidden part of a key or its hash, only
/// whether there was one.
///
/// The application matches on it, so a new event breaks its `match` instead of passing unseen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LimitsEvent {
    /// An attempt was refused: its key is blocked. A flood of attempts on a blocked key is a
    /// flood of these.
    AttemptRefused {
        /// Whose attempt, in which limit.
        key: EventKey,
        /// The moment from which an attempt may pass again.
        until: DateTime<Utc>,
    },
    /// An attempt passed and filled its window: from now on the key is blocked.
    KeyBlocked {
        /// Whose key, in which limit.
        key: EventKey,
        /// The moment from which an attempt may pass again.
        until: DateTime<Utc>,
    },
    /// A block that was still on was lifted before its end.
    BlockLifted {
        /// Whose key, in which limit.
        key: EventKey,
        /// What lifted it.
        by: LiftedBy,
    },
    /// The rule of a limit changed in a tenant. It is for windows opened from now on; windows
    /// that were open keep the rule they were opened with.
    RuleChanged {
        /// The name of the limit.
        limit_name: String,
        /// The tenant.
        tenant_id: Uuid,
        /// The tenant's own rule from now on; `None`: the default from the code is back.
        rule: Option<Rule>,
    },
    /// A pass of the cleanup deleted counters that had been past their end for an hour.
    ExpiredDeleted {
        /// How many.
        counters: u64,
    },
}

/// Whose key an event is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventKey {
    /// The name of the limit.
    pub limit_name: String,
    /// The tenant.
    pub tenant_id: Uuid,
    /// The account the key is about; [`Uuid::nil`] when the key has none.
    pub subject_id: Uuid,
    /// Whether the key has a hidden part, e.g. an e-mail. The part itself is never reported.
    pub has_hidden_part: bool,
}

/// What lifted a block before its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiftedBy {
    /// `Attempts::reset`, e.g. after a sign-in that succeeded.
    Reset,
    /// `Limits::unblock`, by an administrator from the list of blocked keys.
    Unblock,
}
