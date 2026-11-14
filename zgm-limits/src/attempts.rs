//! The attempts of one key in one limit.

use std::fmt;

use uuid::Uuid;

use crate::answer::Answer;
use crate::audit::{EventKey, LiftedBy, LimitsEvent};
use crate::error::LimitsError;
use crate::keyed_hash;
use crate::limits::Limits;
use crate::repository::{self, Hit};
use crate::rule::{Bounds, Rule};

/// How many times `hit` asks the database before it gives up with an error.
const HIT_TRIES: u32 = 10;

/// The attempts of one key in one limit. [`Limits::attempts`] builds it; it keeps the limit,
/// the key and the hashes of the hidden part, so the calls below need no arguments.
pub struct Attempts {
    pub(crate) limits: Limits,
    pub(crate) limit_name: &'static str,
    pub(crate) rule: Rule,
    pub(crate) bounds: Option<Bounds>,
    pub(crate) tenant_id: Uuid,
    pub(crate) subject_id: Uuid,
    pub(crate) hashes: Vec<Vec<u8>>,
}

/// Without the hashes: they stand for the hidden part of the key.
impl fmt::Debug for Attempts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Attempts")
            .field("limit_name", &self.limit_name)
            .field("rule", &self.rule)
            .field("tenant_id", &self.tenant_id)
            .field("subject_id", &self.subject_id)
            .finish_non_exhaustive()
    }
}

impl Attempts {
    /// Counts one attempt, by the rule in force in the tenant (its own for this limit, or the
    /// default from the code), and says whether it passed. A refused attempt changes nothing.
    ///
    /// Call it BEFORE the action the limit guards, e.g. before a password is checked, and do
    /// the action only when the answer is [`Answer::Allowed`]. Checking and counting are one
    /// statement, so of any number of attempts sent at once only as many pass as the window
    /// has room for. Asking first and counting only after a failure would let all of them
    /// through: they would all ask before the first failure is written.
    pub async fn hit(&self) -> Result<Answer, LimitsError> {
        // Every way of counting has a query of its own.
        let (at_most, per) = match self.rule {
            Rule::Window { at_most, per } => (at_most, per),
            Rule::WindowOfDistinct { .. } => {
                return Err(LimitsError::WrongWayOfCounting {
                    limit: self.limit_name,
                    called: "hit",
                    call_instead: "hit_distinct",
                });
            }
            Rule::Bucket { .. } => return Err(LimitsError::BucketNotImplemented),
        };
        // The connection first and the clock second: waiting for a connection must not make
        // the moment of the attempt stale.
        let mut connection = self.limits.pool().acquire().await?;
        let now = self.limits.now();

        // The query gives no answer when other attempts changed the counter between its
        // reading and its writing; asked again, it reads the counter as it is now. One more
        // try is nearly always enough, so a counter that still gives no answer after
        // `HIT_TRIES` is an error and not a loop without an end.
        for _ in 0..HIT_TRIES {
            let hit = repository::hit_window(
                &mut connection,
                self.limit_name,
                self.tenant_id,
                self.subject_id,
                &self.hashes,
                at_most,
                per,
                self.bounds,
                now,
            )
            .await?;
            if let Some(hit) = hit {
                // Back in the pool before the event: the audit of the product must not hold
                // a connection. Dropping it would only ask a task to return it later.
                connection.return_to_pool().await;
                return Ok(self.reported(hit));
            }
        }
        Err(LimitsError::NoAnswer { tries: HIT_TRIES })
    }

    /// Counts one attempt on `element` in a limit that counts distinct elements
    /// ([`Rule::WindowOfDistinct`]), and says whether it passed.
    ///
    /// An element that is already in the window passes and changes nothing; a new one is put
    /// in while there is room, and refused once the window is full. Like [`hit`](Self::hit),
    /// call it BEFORE the action the limit guards.
    ///
    /// The element reaches the database only as a hash, computed by the `KeyedHash` of the
    /// builder. The text is taken exactly as it is.
    ///
    /// ```ignore
    /// // At most ten different accounts tried from one address.
    /// let from_address = limits
    ///     .attempts(AppLimit::AccountsPerAddress)
    ///     .tenant_id(tenant_id)
    ///     .hidden(&ip_address)
    ///     .call()
    ///     .await?;
    /// if let Answer::Refused { until } = from_address.hit_distinct(&login).await? {
    ///     return too_many_attempts(until);
    /// }
    /// ```
    pub async fn hit_distinct(&self, element: &str) -> Result<Answer, LimitsError> {
        let (at_most, per) = match self.rule {
            Rule::WindowOfDistinct { at_most, per } => (at_most, per),
            Rule::Window { .. } => {
                return Err(LimitsError::WrongWayOfCounting {
                    limit: self.limit_name,
                    called: "hit_distinct",
                    call_instead: "hit",
                });
            }
            Rule::Bucket { .. } => return Err(LimitsError::BucketNotImplemented),
        };
        // The hashes of the element, as text: the window keeps them in a list of its state.
        let data = keyed_hash::element_data(self.limit_name, self.tenant_id, element);
        let element_hashes: Vec<String> = self
            .limits
            .hashes_of(&data)
            .await?
            .iter()
            .map(|hash| hash.iter().map(|byte| format!("{byte:02x}")).collect())
            .collect();

        let mut connection = self.limits.pool().acquire().await?;
        let now = self.limits.now();
        for _ in 0..HIT_TRIES {
            let hit = repository::hit_distinct(
                &mut connection,
                self.limit_name,
                self.tenant_id,
                self.subject_id,
                &self.hashes,
                at_most,
                per,
                self.bounds,
                &element_hashes,
                now,
            )
            .await?;
            if let Some(hit) = hit {
                // Back in the pool before the event: the audit of the product must not hold
                // a connection. Dropping it would only ask a task to return it later.
                connection.return_to_pool().await;
                return Ok(self.reported(hit));
            }
        }
        Err(LimitsError::NoAnswer { tries: HIT_TRIES })
    }

    /// Reports what an attempt came to, where a line of a log would be: a refusal, and an
    /// attempt that blocked its key. Gives the answer back.
    fn reported(&self, hit: Hit) -> Answer {
        match (hit.answer, hit.blocked_until) {
            (Answer::Refused { until }, _) => self.limits.emit(LimitsEvent::AttemptRefused {
                key: self.event_key(),
                until,
            }),
            (Answer::Allowed, Some(until)) => self.limits.emit(LimitsEvent::KeyBlocked {
                key: self.event_key(),
                until,
            }),
            (Answer::Allowed, None) => {}
        }
        hit.answer
    }

    /// Forgets the key: deletes its counter, so the key starts from nothing, e.g. after a
    /// sign-in that succeeded. Says whether it lifted a block that was still on.
    pub async fn reset(&self) -> Result<bool, LimitsError> {
        // The connection first and the clock second: waiting for a connection must not make
        // the moment of the attempt stale.
        let mut transaction = self.limits.pool().begin().await?;
        let now = self.limits.now();
        let lifted_block = repository::delete_counter(
            &mut transaction,
            self.limit_name,
            self.tenant_id,
            self.subject_id,
            &self.hashes,
            now,
        )
        .await?;
        // A reset that only forgets a few attempts is everyday work and reports nothing. One
        // that lifts a block changes who may get in, so its event is written in the same
        // transaction: both are committed, or neither.
        if lifted_block {
            let event = LimitsEvent::BlockLifted {
                key: self.event_key(),
                by: LiftedBy::Reset,
            };
            self.limits
                .record_in_transaction(&mut transaction, event)
                .await?;
        }
        transaction.commit().await?;
        Ok(lifted_block)
    }

    /// Whose attempts these are, as events report it: without the hidden part and its hash.
    fn event_key(&self) -> EventKey {
        EventKey {
            limit_name: self.limit_name.to_owned(),
            tenant_id: self.tenant_id,
            subject_id: self.subject_id,
            has_hidden_part: self.hashes.iter().any(|hash| !hash.is_empty()),
        }
    }
}
