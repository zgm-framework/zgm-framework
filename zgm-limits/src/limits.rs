//! The entry point of the module.

use std::borrow::Cow;
use std::fmt;
use std::panic::{self, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use chrono::{DateTime, SubsecRound, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::attempts::Attempts;
use crate::audit::{Audit, AuditError, EventKey, LiftedBy, LimitsEvent};
use crate::blocked::{BlockedCursor, BlockedPage};
use crate::clock::{Clock, SystemClock};
use crate::error::LimitsError;
use crate::jobs::{JobError, OnJobError, StopJobsOnDrop};
use crate::keyed_hash::{self, KeyedHash, MAX_HASH_BYTES};
use crate::repository;
use crate::rule::{self, Bounds, Count, Limit, Period, Rule};
use crate::tenant_rules::{DefaultRule, TenantRule};

/// The module: everything the application does with limits goes through it.
///
/// ```ignore
/// let limits = Limits::builder()
///     .pool(pool)
///     .keyed_hash(hmac)
///     .audit(audit)
///     .limits(AppLimit::VARIANTS)
///     .run_jobs_in_background(|error: JobError| eprintln!("{}: {}", error.job, error.error))
///     .build();
/// ```
///
/// `build()` reads the rule of every limit on the list. A rule cannot hold a value with which
/// the limit would not limit ([`Count`](crate::Count) and [`Period`](crate::Period) panic on
/// zero), so a mistake in a rule stops the application when it starts, not at the first attempt
/// in that limit.
///
/// `build()` starts the loop of the cleanup as a task of the `tokio` runtime, so it must be
/// called inside one. The loop ends when the last clone of the module goes away.
#[derive(Clone)]
pub struct Limits {
    pool: PgPool,
    keyed_hash: Arc<dyn KeyedHash>,
    audit: Arc<dyn Audit>,
    clock: Arc<dyn Clock>,
    /// The limits given to the builder, each with its default rule, in the builder's order.
    defaults: Arc<[DefaultRule]>,
    /// Shared by the clones the application holds; the loop's own copy has none.
    stop_jobs: Option<Arc<StopJobsOnDrop>>,
}

/// Without the keyed hash: it holds secret keys.
impl fmt::Debug for Limits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Limits")
            .field("pool", &self.pool)
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

#[bon::bon]
impl Limits {
    /// Builds the module on the application's pool of connections, after reading the rule of
    /// every limit.
    #[builder]
    pub fn new<L: Limit>(
        pool: PgPool,
        /// All limits of the product, e.g. `AppLimit::VARIANTS` from `strum`. Always required:
        /// the rule of each is read here, so a rule that panics does it now, and an attempt
        /// in a limit that is not on the list is an error.
        limits: &[L],
        /// The hash for the hidden parts of keys. Always required, also when no limit of the
        /// application has a hidden part.
        #[builder(with = |keyed_hash: impl KeyedHash| Arc::new(keyed_hash) as Arc<dyn KeyedHash>)]
        keyed_hash: Arc<dyn KeyedHash>,
        /// Where the module reports what it did: refused attempts, blocks, changed rules.
        /// Always required; the module has no logging of its own.
        #[builder(with = |audit: impl Audit| Arc::new(audit) as Arc<dyn Audit>)]
        audit: Arc<dyn Audit>,
        /// A clock other than the system one, for tests that move time.
        #[builder(
            default = Arc::new(SystemClock),
            with = |clock: impl Clock| Arc::new(clock) as Arc<dyn Clock>,
        )]
        clock: Arc<dyn Clock>,
        /// Starts the loop of the cleanup, which deletes the counters that mean nothing any
        /// more. Always required: the application calls no job itself. The function gets
        /// every error of the loop; the module logs nothing, and the loop goes on.
        #[builder(with = |on_error: impl Fn(JobError) + Send + Sync + 'static| Box::new(on_error) as OnJobError)]
        run_jobs_in_background: OnJobError,
    ) -> Self {
        // Before anything is started: reading a rule or bounds with a wrong value panics, here
        // and not at the first attempt in that limit.
        let defaults: Vec<DefaultRule> = limits
            .iter()
            .map(|&limit| {
                let default = DefaultRule {
                    limit_name: limit.into(),
                    rule: limit.rule(),
                    bounds: Bounds::of(limit),
                };
                rule::check_distinct_elements(default.limit_name, default.rule, default.bounds);
                default
            })
            .collect();

        let mut limits = Self {
            pool,
            keyed_hash,
            audit,
            clock,
            defaults: defaults.into(),
            stop_jobs: None,
        };
        // The loop takes its copy before the handle is set, so that it does not hold it.
        let task = tokio::spawn(limits.clone().run_jobs(run_jobs_in_background));
        limits.stop_jobs = Some(Arc::new(StopJobsOnDrop(task.abort_handle())));
        limits
    }

    /// The default rule of a limit with its bounds, or an error for a limit the builder did
    /// not get.
    fn default_of(&self, limit_name: &'static str) -> Result<DefaultRule, LimitsError> {
        self.defaults
            .iter()
            .find(|default| default.limit_name == limit_name)
            .copied()
            .ok_or(LimitsError::UnknownLimit { limit: limit_name })
    }

    /// Reports a change to the application's [`Audit`], inside the transaction that makes the
    /// change. An error of the audit, or a panic in it, is the error of the operation: the
    /// caller returns it, and the transaction is rolled back.
    pub(crate) async fn record_in_transaction(
        &self,
        transaction: &mut PgConnection,
        event: LimitsEvent,
    ) -> Result<(), LimitsError> {
        let recording = self.audit.record_in_transaction(transaction, event);
        match CatchPanic(recording).await {
            Ok(recorded) => Ok(recorded?),
            Err(Panicked) => Err(AuditError::new("the audit panicked").into()),
        }
    }

    /// Reports to the application's [`Audit`] something that changed nothing an event could
    /// be written together with: a refusal, a key that an attempt blocked, a pass of the
    /// cleanup. A panic in it is caught and dropped: this reporting must never break counting
    /// or the cleanup.
    pub(crate) fn emit(&self, event: LimitsEvent) {
        let _panic =
            panic::catch_unwind(AssertUnwindSafe(|| self.audit.record_after_the_fact(event)));
    }

    /// The hashes of `data` from the application's `KeyedHash`, one for every version of the
    /// secret key that can still be read, checked to be what the trait promises.
    pub(crate) async fn hashes_of(&self, data: &[u8]) -> Result<Vec<Vec<u8>>, LimitsError> {
        let hashes = self.keyed_hash.hashes(data).await?;
        // An empty list has no hash to write with, and an empty hash is how the table marks a
        // key with no hidden part.
        if hashes.is_empty() || hashes.iter().any(Vec::is_empty) {
            return Err(LimitsError::KeyedHashGaveNoHash);
        }
        // A hash goes into the primary key, where the database refuses a long entry: the
        // attempt would fail there and never be counted. Refuse it here instead.
        if hashes.iter().any(|hash| hash.len() > MAX_HASH_BYTES) {
            return Err(LimitsError::KeyedHashGaveLongHash);
        }
        Ok(hashes)
    }

    /// The pool the module runs its queries on.
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The moment of an attempt, cut to microseconds: PostgreSQL keeps no more, so a moment
    /// written to the database and read back is the same moment.
    pub(crate) fn now(&self) -> DateTime<Utc> {
        self.clock.now().trunc_subsecs(6)
    }

    /// The attempts of one key in one limit. Built once, it counts with
    /// [`hit`](Attempts::hit) and forgets with [`reset`](Attempts::reset).
    ///
    /// The limit and the tenant are always given. What follows says whose attempts they are;
    /// with nothing more, they are the attempts of the whole tenant.
    ///
    /// ```ignore
    /// let login_attempts = limits
    ///     .attempts(AppLimit::WrongPassword)
    ///     .tenant_id(tenant_id)
    ///     .subject_id(account_id)
    ///     .call()
    ///     .await?;
    ///
    /// // Count first: a refused attempt does not get its password checked.
    /// if let Answer::Refused { until } = login_attempts.hit().await? {
    ///     return too_many_attempts(until);
    /// }
    /// if password_is_right {
    ///     login_attempts.reset().await?;
    /// }
    /// ```
    ///
    /// The hash of a hidden part is computed here, once, and not by every call that follows.
    #[builder]
    pub async fn attempts<L: Limit>(
        &self,
        #[builder(start_fn)] limit: L,
        /// The tenant.
        tenant_id: Uuid,
        /// The account, or another thing with a UUID, the attempts are about. It lies in the
        /// database in the open. Not the nil UUID: for the attempts of the whole tenant, give
        /// no account at all.
        subject_id: Option<Uuid>,
        /// The hidden part of the key, e.g. an e-mail or an IP address. It reaches the
        /// database only as a hash, computed by the `KeyedHash` of the builder.
        ///
        /// The text is made uniform first: letters go to lower case, and all white space and
        /// invisible characters, such as the zero-width space, are removed, so `Anna@Firma.pl `, `anna @firma.pl` and `anna@firma.pl` are one key and
        /// a change of case does not get round the limit.
        hidden: Option<&str>,
        /// A hidden part taken exactly as it is, e.g. an identifier in which the case of
        /// letters matters. Instead of `hidden`, not together with it.
        hidden_exact: Option<&str>,
    ) -> Result<Attempts, LimitsError> {
        let limit_name: &'static str = limit.into();
        // Only the limits given to the builder had their rules read when the module was built.
        let default = self.default_of(limit_name)?;
        let hidden: Option<Cow<'_, str>> = match (hidden, hidden_exact) {
            (Some(text), None) => Some(Cow::Owned(keyed_hash::made_uniform(text))),
            (None, Some(text)) => Some(Cow::Borrowed(text)),
            (None, None) => None,
            // Which of the two is the key? Picking one silently would count on a counter the
            // caller did not mean.
            (Some(_), Some(_)) => return Err(LimitsError::HiddenAndHiddenExact),
        };
        // The nil UUID is how the table marks a key with no account. Given as an account, it
        // would silently become the counter of the whole tenant, and one unknown account
        // would block everybody.
        if subject_id == Some(Uuid::nil()) {
            return Err(LimitsError::NilSubject);
        }
        // A key with no account has the nil UUID there, and a key with no hidden part one
        // empty hash: the table keeps no NULL in the key.
        let subject_id = subject_id.unwrap_or(Uuid::nil());
        let hashes = match hidden {
            Some(hidden) => {
                let data = keyed_hash::key_data(limit_name, tenant_id, subject_id, &hidden);
                self.hashes_of(&data).await?
            }
            None => vec![Vec::new()],
        };

        Ok(Attempts {
            limits: self.clone(),
            limit_name,
            rule: default.rule,
            bounds: default.bounds,
            tenant_id,
            subject_id,
            hashes,
        })
    }

    /// Lists one page of the blocked keys of one tenant, the newest block first: the keys of
    /// which no attempt passes now. A page holds at most 100 keys.
    ///
    /// ```ignore
    /// let page = limits
    ///     .get_blocked()
    ///     .tenant_id(tenant_id)
    ///     .limit_name(AppLimit::Export)
    ///     .call()
    ///     .await?;
    /// let next = limits
    ///     .get_blocked()
    ///     .tenant_id(tenant_id)
    ///     .maybe_after(page.next_cursor)
    ///     .call()
    ///     .await?;
    /// ```
    #[builder]
    pub async fn get_blocked(
        &self,
        /// The tenant whose blocked keys to list.
        tenant_id: Uuid,
        /// Only the keys of this limit.
        #[builder(into)]
        limit_name: Option<&'static str>,
        /// `true`: only the counters of a whole tenant, with neither an account nor a hidden
        /// part. `false`: only the others.
        whole_tenant: Option<bool>,
        /// Also the keys whose block is over but whose row is still in the database.
        #[builder(default)]
        include_expired: bool,
        /// `next_cursor` of the page before; without it, the first page.
        after: Option<BlockedCursor>,
    ) -> Result<BlockedPage, LimitsError> {
        let mut connection = self.pool.acquire().await?;
        let now = self.now();
        Ok(repository::list_blocked(
            &mut connection,
            tenant_id,
            limit_name,
            whole_tenant,
            include_expired,
            after,
            now,
        )
        .await?)
    }

    /// Unblocks the key `id` from the list of the blocked keys of `tenant_id`: deletes its
    /// counter, so the key starts from nothing. Says whether that tenant had such a blocked
    /// key; `false` also for an id of another tenant, and when somebody unblocked the key
    /// first or its row was already cleaned up.
    pub async fn unblock(&self, tenant_id: Uuid, id: i64) -> Result<bool, LimitsError> {
        let mut transaction = self.pool.begin().await?;
        let now = self.now();
        let Some(deleted) =
            repository::delete_blocked(&mut transaction, tenant_id, id, now).await?
        else {
            return Ok(false);
        };
        // A block whose time was already over was lifted by nobody.
        if deleted.was_still_blocked {
            let event = LimitsEvent::BlockLifted {
                key: EventKey {
                    limit_name: deleted.limit_name,
                    tenant_id,
                    subject_id: deleted.subject_id,
                    has_hidden_part: deleted.has_hidden_part,
                },
                by: LiftedBy::Unblock,
            };
            self.record_in_transaction(&mut transaction, event).await?;
        }
        transaction.commit().await?;
        Ok(true)
    }

    /// All limits with their default rules, the ones from the code, in the order the builder
    /// got them. No query: the defaults are not in the database.
    pub fn get_defaults(&self) -> &[DefaultRule] {
        &self.defaults
    }

    /// All limits with the rule in force in one tenant: its own where it set one, the default
    /// from the code elsewhere. In the order the builder got the limits.
    #[builder]
    pub async fn get_rules(
        &self,
        /// The tenant.
        tenant_id: Uuid,
    ) -> Result<Vec<TenantRule>, LimitsError> {
        let mut connection = self.pool.acquire().await?;
        let own = repository::list_tenant_rules(&mut connection, tenant_id).await?;

        // A row of a limit that is no longer in the code is passed over.
        Ok(self
            .defaults
            .iter()
            .map(|default| {
                let own_rule = own
                    .iter()
                    .find(|(limit_name, _)| limit_name == default.limit_name)
                    .map(|(_, rule)| *rule);
                // A row written before the bounds in the code got narrower, or went away, is
                // held to the bounds of today, as the counting holds it.
                let own_rule = own_rule.map(|rule| within(rule, *default));
                TenantRule {
                    limit_name: default.limit_name,
                    rule: own_rule.unwrap_or(default.rule),
                    overridden: own_rule.is_some(),
                    bounds: default.bounds,
                }
            })
            .collect())
    }

    /// Sets the tenant's own rule for a limit, in place of the default. Says whether anything
    /// changed; the same rule set again changes nothing.
    ///
    /// **The new rule is for windows opened from now on, on every server.** A window that is
    /// open keeps counting by the rule it was opened with, to its end: nothing is deleted and
    /// no block is lifted. To let a blocked key through at once, unblock it from the list.
    ///
    /// The rule must be within the [`Bounds`] the code gives the limit; a limit with no bounds
    /// cannot be changed.
    ///
    /// Who may call it is the application's decision.
    ///
    /// ```ignore
    /// limits
    ///     .override_rule(AppLimit::WrongPassword)
    ///     .tenant_id(tenant_id)
    ///     .rule(Rule::Window { at_most: Count::new(10), per: Period::hours(1) })
    ///     .call()
    ///     .await?;
    /// ```
    #[builder]
    pub async fn override_rule<L: Limit>(
        &self,
        #[builder(start_fn)] limit: L,
        /// The tenant.
        tenant_id: Uuid,
        /// The rule to count by from now on, with its way of counting.
        rule: Rule,
    ) -> Result<bool, LimitsError> {
        let limit_name: &'static str = limit.into();
        let default = self.default_of(limit_name)?;
        // A tenant must not get a rule every attempt would fail on.
        let Some((kind, at_most, per)) = rule.kind_and_numbers() else {
            return Err(LimitsError::BucketNotImplemented);
        };
        // Whether the limit counts attempts or distinct elements is in the code, which calls
        // `hit` or `hit_distinct`; a tenant changes the numbers only.
        if default.rule.kind_and_numbers().map(|(kind, ..)| kind) != Some(kind) {
            return Err(LimitsError::RuleOfAnotherKind { limit: limit_name });
        }
        // What the tenant may set comes from the code: no bounds there, no change here.
        let Some(bounds) = default.bounds else {
            return Err(LimitsError::RuleCannotBeChanged { limit: limit_name });
        };
        if let Some((field, value, lowest, highest)) = bounds.problem_with(rule) {
            return Err(LimitsError::RuleOutOfBounds {
                limit: limit_name,
                field,
                value,
                lowest,
                highest,
            });
        }

        // Only the rule is written. Windows that are open keep counting by the rule they were
        // opened with, which lies next to their count; the new rule is for windows opened from
        // now on.
        let mut transaction = self.pool.begin().await?;
        let changed = repository::set_tenant_rule(
            &mut transaction,
            tenant_id,
            limit_name,
            kind,
            at_most,
            per,
        )
        .await?;
        if changed {
            let event = LimitsEvent::RuleChanged {
                limit_name: limit_name.to_owned(),
                tenant_id,
                rule: Some(rule),
            };
            self.record_in_transaction(&mut transaction, event).await?;
        }
        transaction.commit().await?;
        Ok(changed)
    }

    /// Deletes the tenant's own rule for a limit, so the default from the code is in force
    /// again. Says whether the tenant had one.
    ///
    /// Like every change of a rule, it is for windows opened from now on; open windows keep
    /// the rule they were opened with.
    #[builder]
    pub async fn restore_default<L: Limit>(
        &self,
        #[builder(start_fn)] limit: L,
        /// The tenant.
        tenant_id: Uuid,
    ) -> Result<bool, LimitsError> {
        let limit_name: &'static str = limit.into();
        self.default_of(limit_name)?;

        let mut transaction = self.pool.begin().await?;
        let had_own_rule =
            repository::delete_tenant_rule(&mut transaction, tenant_id, limit_name).await?;
        if had_own_rule {
            let event = LimitsEvent::RuleChanged {
                limit_name: limit_name.to_owned(),
                tenant_id,
                rule: None,
            };
            self.record_in_transaction(&mut transaction, event).await?;
        }
        transaction.commit().await?;
        Ok(had_own_rule)
    }
}

/// A tenant's own rule held to the bounds of the limit: each number moved to the nearest
/// allowed one. With no bounds the tenant may change nothing, and a rule that counts another
/// way than the default is not the limit's rule at all, so in both cases the default stands.
fn within(own: Rule, default: DefaultRule) -> Rule {
    let (Some((kind, at_most, per)), Some((default_kind, ..)), Some(bounds)) = (
        own.kind_and_numbers(),
        default.rule.kind_and_numbers(),
        default.bounds,
    ) else {
        return default.rule;
    };
    if kind != default_kind {
        return default.rule;
    }
    let at_most = Count::new(
        at_most
            .get()
            .clamp(bounds.min_at_most.get(), bounds.max_at_most.get()),
    );
    let per = Period::seconds(
        per.as_seconds()
            .clamp(bounds.min_per.as_seconds(), bounds.max_per.as_seconds()),
    );
    Rule::of_kind(kind, at_most, per).unwrap_or(default.rule)
}

/// The audit panicked while it was taking an event.
struct Panicked;

/// The future of the application's audit, with a panic in it turned into a value: reporting
/// must not be able to end the task that asked, least of all the loop of the cleanup.
struct CatchPanic<'a>(Pin<Box<dyn Future<Output = Result<(), AuditError>> + Send + 'a>>);

impl Future for CatchPanic<'_> {
    type Output = Result<Result<(), AuditError>, Panicked>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match panic::catch_unwind(AssertUnwindSafe(|| self.0.as_mut().poll(context))) {
            Ok(Poll::Ready(recorded)) => Poll::Ready(Ok(recorded)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(_panic) => Poll::Ready(Err(Panicked)),
        }
    }
}
