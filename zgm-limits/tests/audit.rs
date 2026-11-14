//! The events the module reports to the application's `Audit`.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::{HOUR, RecordedAudit, TestClock, TestHash, TestLimit, all_limits, counters, noon};
use sqlx::{PgConnection, PgPool};
use tokio::task::JoinSet;
use uuid::Uuid;
use zgm_limits::{
    Answer, Attempts, Audit, AuditError, Count, EventKey, LiftedBy, Limits, LimitsError,
    LimitsEvent, Period, Rule,
};

/// The module with an audit that keeps the events and a clock that stands at noon.
fn recorded(pool: &PgPool) -> (Limits, RecordedAudit, TestClock) {
    let audit = RecordedAudit::default();
    let clock = TestClock::at_noon();
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(TestHash::new())
        .audit(audit.clone())
        .clock(clock.clone())
        .limits(all_limits())
        .run_jobs_in_background(|_error| {})
        .build();
    (limits, audit, clock)
}

fn key(limit_name: &str, tenant_id: Uuid, subject_id: Uuid, has_hidden_part: bool) -> EventKey {
    EventKey {
        limit_name: limit_name.to_owned(),
        tenant_id,
        subject_id,
        has_hidden_part,
    }
}

#[sqlx::test]
async fn an_attempt_reports_a_block_and_every_refusal_and_nothing_else(pool: PgPool) {
    let (limits, audit, _clock) = recorded(&pool);
    let tenant = Uuid::new_v4();
    let account = Uuid::new_v4();
    let attempts = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .subject_id(account)
        .hidden("anna@firma.pl")
        .call()
        .await
        .unwrap();
    let whose = key("three_per_hour", tenant, account, true);
    let until = noon() + HOUR;

    // Two attempts with room left: ordinary work, no event.
    let _counted = attempts.hit().await.unwrap();
    let _counted = attempts.hit().await.unwrap();
    assert_eq!(audit.take(), []);

    // The third fills the window: the key is blocked from now on.
    let _counted = attempts.hit().await.unwrap();
    assert_eq!(
        audit.take(),
        [LimitsEvent::KeyBlocked {
            key: whose.clone(),
            until
        }]
    );

    // Every refused attempt is reported: this is how a flood on a blocked key shows.
    let _refused = attempts.hit().await.unwrap();
    let _refused = attempts.hit().await.unwrap();
    let refused = LimitsEvent::AttemptRefused { key: whose, until };
    assert_eq!(audit.take(), [refused.clone(), refused]);
}

#[sqlx::test]
async fn an_event_never_holds_the_hidden_part_or_its_hash(pool: PgPool) {
    let (limits, audit, _clock) = recorded(&pool);
    let attempts = limits
        .attempts(TestLimit::OnePerHour)
        .tenant_id(Uuid::new_v4())
        .hidden("anna@firma.pl")
        .call()
        .await
        .unwrap();
    let _counted = attempts.hit().await.unwrap();
    let _refused = attempts.hit().await.unwrap();
    assert!(attempts.reset().await.unwrap());
    let hash = hex(&counters_hash_of_a_new_attempt(&pool, &limits).await);

    let printed = format!("{:?}", audit.take());

    assert!(printed.contains("has_hidden_part: true"));
    assert!(!printed.contains("anna"));
    assert!(!printed.contains(&hash));
}

/// The hash the same hidden part gets in the database, to look for in the events.
async fn counters_hash_of_a_new_attempt(pool: &PgPool, limits: &Limits) -> Vec<u8> {
    let _counted = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(Uuid::new_v4())
        .hidden("anna@firma.pl")
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    counters(pool).await.remove(0).0
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[sqlx::test]
async fn a_block_lifted_before_its_end_is_reported_with_what_lifted_it(pool: PgPool) {
    let (limits, audit, clock) = recorded(&pool);
    let tenant = Uuid::new_v4();
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let third = Uuid::new_v4();
    let attempts_of = async |account: Uuid| {
        limits
            .attempts(TestLimit::OnePerHour)
            .tenant_id(tenant)
            .subject_id(account)
            .call()
            .await
            .unwrap()
    };
    for account in [first, second, third] {
        let _counted = attempts_of(account).await.hit().await.unwrap();
    }
    audit.take();

    // A reset that only forgets attempts reports nothing; one that lifts a block does.
    let not_blocked = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap();
    let _counted = not_blocked.hit().await.unwrap();
    assert!(!not_blocked.reset().await.unwrap());
    assert_eq!(audit.take(), []);
    assert!(attempts_of(first).await.reset().await.unwrap());
    assert_eq!(
        audit.take(),
        [LimitsEvent::BlockLifted {
            key: key("one_per_hour", tenant, first, false),
            by: LiftedBy::Reset,
        }]
    );

    // An administrator unblocks the second from the list.
    let blocked = limits.get_blocked().tenant_id(tenant).call().await.unwrap();
    let id_of = |account: Uuid| {
        blocked
            .items
            .iter()
            .find(|blocked| blocked.subject_id == account)
            .unwrap()
            .id
    };
    assert!(limits.unblock(tenant, id_of(second)).await.unwrap());
    assert_eq!(
        audit.take(),
        [LimitsEvent::BlockLifted {
            key: key("one_per_hour", tenant, second, false),
            by: LiftedBy::Unblock,
        }]
    );
    // An unblock that deletes nothing, here with the id of another tenant, reports nothing.
    assert!(!limits.unblock(Uuid::new_v4(), id_of(third)).await.unwrap());
    assert_eq!(audit.take(), []);

    // The block of the third ends on its own; deleting its row afterwards lifts nothing.
    clock.advance(HOUR);
    assert!(limits.unblock(tenant, id_of(third)).await.unwrap());
    assert_eq!(audit.take(), []);
}

#[sqlx::test]
async fn a_change_of_a_rule_is_reported(pool: PgPool) {
    let (limits, audit, _clock) = recorded(&pool);
    let tenant = Uuid::new_v4();
    for _ in 0..2 {
        let _counted = limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .subject_id(Uuid::new_v4())
            .call()
            .await
            .unwrap()
            .hit()
            .await
            .unwrap();
    }
    let rule = Rule::Window {
        at_most: Count::new(5),
        per: Period::hours(2),
    };
    let set = async || {
        limits
            .override_rule(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .rule(rule)
            .call()
            .await
            .unwrap()
    };
    let restore = async || {
        limits
            .restore_default(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .call()
            .await
            .unwrap()
    };

    assert!(set().await);
    assert_eq!(
        audit.take(),
        [LimitsEvent::RuleChanged {
            limit_name: "three_per_hour".to_owned(),
            tenant_id: tenant,
            rule: Some(rule),
        }]
    );
    // The same rule again changes nothing and reports nothing.
    assert!(!set().await);
    assert_eq!(audit.take(), []);

    assert!(restore().await);
    assert_eq!(
        audit.take(),
        [LimitsEvent::RuleChanged {
            limit_name: "three_per_hour".to_owned(),
            tenant_id: tenant,
            rule: None,
        }]
    );
    assert!(!restore().await);
    assert_eq!(audit.take(), []);

    // A rule that is refused is not a change.
    let refused = limits
        .override_rule(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .rule(Rule::Window {
            at_most: Count::new(1000),
            per: Period::hours(1),
        })
        .call()
        .await;
    assert!(refused.is_err());
    assert_eq!(audit.take(), []);
}

#[sqlx::test]
async fn a_pass_of_the_cleanup_reports_how_many_counters_it_deleted(pool: PgPool) {
    // 150 counters that ended two hours before noon: a full batch and a half.
    sqlx::query(
        "INSERT INTO zgm_limits.counters \
             (limit_name, tenant_id, subject_id, key_hash, state, expires_at) \
         SELECT 'three_per_hour', uuidv7(), '00000000-0000-0000-0000-000000000000', '', \
                '{\"count\": 1}', $1 \
         FROM generate_series(1, 150)",
    )
    .bind(noon() - HOUR - HOUR)
    .execute(&pool)
    .await
    .unwrap();

    // Building starts the first pass at once.
    let (_limits, audit, _clock) = recorded(&pool);

    // One event for the whole pass, with both batches in it.
    let mut events = Vec::new();
    for _ in 0..500 {
        events.extend(audit.take());
        if !events.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(events, [LimitsEvent::ExpiredDeleted { counters: 150 }]);

    // A pass that finds nothing reports nothing.
    let (_again, audit_of_the_second, _clock) = recorded(&pool);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(audit_of_the_second.take(), []);
}

/// An audit that fails every time.
struct PanickingAudit;

#[async_trait]
impl Audit for PanickingAudit {
    async fn record_in_transaction(
        &self,
        _transaction: &mut PgConnection,
        _event: LimitsEvent,
    ) -> Result<(), AuditError> {
        panic!("the audit itself fails");
    }

    fn record_after_the_fact(&self, _event: LimitsEvent) {
        panic!("the audit itself fails");
    }
}

#[sqlx::test]
async fn a_panic_in_the_audit_breaks_no_attempt_and_takes_a_change_back(pool: PgPool) {
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(TestHash::new())
        .audit(PanickingAudit)
        .limits(all_limits())
        .run_jobs_in_background(|_error| {})
        .build();
    let tenant = Uuid::new_v4();
    let attempts = limits
        .attempts(TestLimit::OnePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap();

    // The first attempt blocks and the second is refused: both report, and both reports panic.
    assert_eq!(attempts.hit().await.unwrap(), Answer::Allowed);
    assert!(matches!(
        attempts.hit().await.unwrap(),
        Answer::Refused { .. }
    ));
    // A reset that lifts the block reports inside its transaction: there the panic is the
    // error of the reset, and the key stays blocked.
    assert!(matches!(attempts.reset().await, Err(LimitsError::Audit(_))));
    assert!(matches!(
        attempts.hit().await.unwrap(),
        Answer::Refused { .. }
    ));
}

#[sqlx::test]
async fn attempts_at_once_report_one_block_and_a_refusal_for_each_of_the_rest(pool: PgPool) {
    let (limits, audit, _clock) = recorded(&pool);
    let tenant = Uuid::new_v4();

    let mut attempts = JoinSet::new();
    for _ in 0..30 {
        let limits = limits.clone();
        attempts.spawn(async move {
            limits
                .attempts(TestLimit::ThreePerHour)
                .tenant_id(tenant)
                .call()
                .await
                .unwrap()
                .hit()
                .await
                .unwrap()
        });
    }
    attempts.join_all().await;

    let events = audit.take();
    let blocks = events
        .iter()
        .filter(|event| matches!(event, LimitsEvent::KeyBlocked { .. }))
        .count();
    let refusals = events
        .iter()
        .filter(|event| matches!(event, LimitsEvent::AttemptRefused { .. }))
        .count();
    // Three passed, and only the one that filled the window blocked; 27 were refused.
    assert_eq!((blocks, refusals, events.len()), (1, 27, 28));
}

/// An audit that notes, for every event, how many connections of the pool lay unused at the
/// moment the event was reported.
struct PoolWatchingAudit {
    pool: sqlx::PgPool,
    idle_at_each_event: Arc<Mutex<Vec<usize>>>,
}

#[async_trait]
impl Audit for PoolWatchingAudit {
    async fn record_in_transaction(
        &self,
        _transaction: &mut PgConnection,
        _event: LimitsEvent,
    ) -> Result<(), AuditError> {
        // Inside the transaction the connection is in use, as it must be.
        Ok(())
    }

    fn record_after_the_fact(&self, _event: LimitsEvent) {
        self.idle_at_each_event
            .lock()
            .unwrap()
            .push(self.pool.num_idle());
    }
}

#[sqlx::test]
async fn the_connection_is_back_in_the_pool_before_an_event_is_reported_after_the_fact(
    _pool_options: sqlx::postgres::PgPoolOptions,
    connect_options: sqlx::postgres::PgConnectOptions,
) {
    // A pool of one connection: when an event is reported, that one connection is either
    // back in the pool or still held by the call that reports.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options)
        .await
        .unwrap();
    let idle_at_each_event = Arc::new(Mutex::new(Vec::new()));
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(TestHash::new())
        .audit(PoolWatchingAudit {
            pool,
            idle_at_each_event: idle_at_each_event.clone(),
        })
        .limits(all_limits())
        .run_jobs_in_background(|_error| {})
        .build();
    let tenant = Uuid::new_v4();

    // An attempt that blocks its key, a refusal, and a reset that lifts the block.
    let attempts = limits
        .attempts(TestLimit::OnePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap();
    assert_eq!(attempts.hit().await.unwrap(), Answer::Allowed);
    assert!(matches!(
        attempts.hit().await.unwrap(),
        Answer::Refused { .. }
    ));
    assert!(attempts.reset().await.unwrap());

    // The same for a window of distinct elements, and an unblock from the list.
    let distinct = limits
        .attempts(TestLimit::ThreeDistinctPerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap();
    for element in ["a", "b", "c"] {
        assert_eq!(
            distinct.hit_distinct(element).await.unwrap(),
            Answer::Allowed
        );
    }
    assert!(matches!(
        distinct.hit_distinct("d").await.unwrap(),
        Answer::Refused { .. }
    ));
    let blocked = limits.get_blocked().tenant_id(tenant).call().await.unwrap();
    assert!(limits.unblock(tenant, blocked.items[0].id).await.unwrap());

    // A change of a rule and the way back to the default.
    let stricter = Rule::Window {
        at_most: Count::new(2),
        per: Period::hours(1),
    };
    assert!(
        limits
            .override_rule(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .rule(stricter)
            .call()
            .await
            .unwrap()
    );
    assert!(
        limits
            .restore_default(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .call()
            .await
            .unwrap()
    );

    // Four events came after the fact, two blocks and two refusals, and the connection lay in
    // the pool at every one of them. The other four came inside their transactions.
    assert_eq!(*idle_at_each_event.lock().unwrap(), [1; 4]);
}

/// An audit that writes the event of every change into a table of the application, with the
/// transaction it is given, and then does what the test tells it to: nothing more, fail,
/// panic, or never end.
#[derive(Clone, Default)]
struct TableAudit(Arc<Mutex<Then>>);

#[derive(Clone, Copy, Default, PartialEq)]
enum Then {
    #[default]
    Succeed,
    Fail,
    Panic,
    NeverEnd,
}

impl TableAudit {
    fn then(&self, then: Then) {
        *self.0.lock().unwrap() = then;
    }
}

#[async_trait]
impl Audit for TableAudit {
    async fn record_in_transaction(
        &self,
        transaction: &mut PgConnection,
        event: LimitsEvent,
    ) -> Result<(), AuditError> {
        sqlx::query("INSERT INTO app_audit (event) VALUES ($1)")
            .bind(format!("{event:?}"))
            .execute(transaction)
            .await
            .map_err(AuditError::new)?;
        let then = *self.0.lock().unwrap();
        match then {
            Then::Succeed => Ok(()),
            Then::Fail => Err(AuditError::new("the audit log is full")),
            Then::Panic => panic!("the audit itself fails"),
            Then::NeverEnd => std::future::pending().await,
        }
    }

    fn record_after_the_fact(&self, _event: LimitsEvent) {}
}

/// The module with an audit that writes to the table `app_audit` of the same database, a
/// tenant, and an account of it that one attempt has just blocked.
async fn with_table_audit(pool: &PgPool) -> (Limits, TableAudit, Uuid, Attempts) {
    sqlx::query("CREATE TABLE app_audit (event text NOT NULL)")
        .execute(pool)
        .await
        .unwrap();
    let audit = TableAudit::default();
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(TestHash::new())
        .audit(audit.clone())
        .limits(all_limits())
        .run_jobs_in_background(|_error| {})
        .build();
    let tenant = Uuid::new_v4();
    let attempts = limits
        .attempts(TestLimit::OnePerHour)
        .tenant_id(tenant)
        .subject_id(Uuid::new_v4())
        .call()
        .await
        .unwrap();
    assert_eq!(attempts.hit().await.unwrap(), Answer::Allowed);
    (limits, audit, tenant, attempts)
}

/// The events the application's table holds.
async fn audit_rows(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT event FROM app_audit")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// A rule a tenant may set for `ThreePerHour`.
fn two_per_hour() -> Rule {
    Rule::Window {
        at_most: Count::new(2),
        per: Period::hours(1),
    }
}

#[sqlx::test]
async fn the_event_of_a_change_is_written_in_the_transaction_of_that_change(pool: PgPool) {
    let (limits, _audit, tenant, attempts) = with_table_audit(&pool).await;
    // The block itself came after the fact, and this audit writes only changes.
    assert_eq!(audit_rows(&pool).await.len(), 0);

    // A reset that lifts the block, an unblock from the list, a change of a rule and the way
    // back: one row each, there as soon as the call returns.
    assert!(attempts.reset().await.unwrap());
    assert_eq!(audit_rows(&pool).await.len(), 1);

    assert_eq!(attempts.hit().await.unwrap(), Answer::Allowed);
    let blocked = limits.get_blocked().tenant_id(tenant).call().await.unwrap();
    assert!(limits.unblock(tenant, blocked.items[0].id).await.unwrap());
    assert_eq!(audit_rows(&pool).await.len(), 2);

    assert!(
        limits
            .override_rule(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .rule(two_per_hour())
            .call()
            .await
            .unwrap()
    );
    assert!(
        limits
            .restore_default(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .call()
            .await
            .unwrap()
    );
    let rows = audit_rows(&pool).await;
    assert_eq!(rows.len(), 4);
    assert!(rows[0].starts_with("BlockLifted") && rows[0].contains("Reset"));
    assert!(rows[1].starts_with("BlockLifted") && rows[1].contains("Unblock"));
    assert!(rows[2].starts_with("RuleChanged") && rows[3].starts_with("RuleChanged"));
}

#[sqlx::test]
async fn an_audit_that_fails_takes_the_change_back_with_it(pool: PgPool) {
    let (limits, audit, tenant, attempts) = with_table_audit(&pool).await;
    let blocked = limits.get_blocked().tenant_id(tenant).call().await.unwrap();

    // Nothing that lifts the block or changes a rule goes through without its event, whether
    // the audit returns an error or panics.
    for then in [Then::Fail, Then::Panic] {
        audit.then(then);
        assert!(matches!(
            limits.unblock(tenant, blocked.items[0].id).await,
            Err(LimitsError::Audit(_))
        ));
        assert!(matches!(attempts.reset().await, Err(LimitsError::Audit(_))));
        assert!(matches!(
            limits
                .override_rule(TestLimit::ThreePerHour)
                .tenant_id(tenant)
                .rule(two_per_hour())
                .call()
                .await,
            Err(LimitsError::Audit(_))
        ));

        // The key is still blocked, the rule is still the default, and no event was written.
        let still = limits.get_blocked().tenant_id(tenant).call().await.unwrap();
        assert_eq!(still.items.len(), 1);
        let rules = limits.get_rules().tenant_id(tenant).call().await.unwrap();
        assert!(rules.iter().all(|rule| !rule.overridden));
        assert_eq!(audit_rows(&pool).await.len(), 0);
    }

    // Attempts do not go through that audit: they are still counted and refused.
    assert!(matches!(
        attempts.hit().await.unwrap(),
        Answer::Refused { .. }
    ));
}

#[sqlx::test]
async fn a_request_dropped_while_its_event_is_written_leaves_nothing_behind(pool: PgPool) {
    let (limits, audit, tenant, _attempts) = with_table_audit(&pool).await;
    let blocked = limits.get_blocked().tenant_id(tenant).call().await.unwrap();

    // The unblock has deleted the row and written its event, and then the caller goes away,
    // as a request does when its client disconnects.
    audit.then(Then::NeverEnd);
    let dropped = tokio::time::timeout(
        Duration::from_millis(300),
        limits.unblock(tenant, blocked.items[0].id),
    )
    .await;
    assert!(dropped.is_err());

    // Neither happened: the key is still blocked, and no event says it was unblocked.
    audit.then(Then::Succeed);
    let still = limits.get_blocked().tenant_id(tenant).call().await.unwrap();
    assert_eq!(still.items.len(), 1);
    assert_eq!(audit_rows(&pool).await.len(), 0);
}
