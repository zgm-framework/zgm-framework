//! `Attempts::hit_distinct`: a window filled by distinct elements, on a real database.

mod common;

use std::time::Duration;

use common::{
    HOUR, MINUTE, NoAudit, RecordedAudit, TestClock, TestHash, TestLimit, all_limits, counters,
    limits, limits_at_noon, limits_with_hash, noon,
};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::task::JoinSet;
use uuid::Uuid;
use zgm_limits::{Answer, Attempts, Count, Limits, LimitsError, LimitsEvent, Period, Rule};

/// The attempts from one address in the limit of three different elements an hour.
async fn from_address(limits: &Limits, tenant: Uuid) -> Attempts {
    limits
        .attempts(TestLimit::ThreeDistinctPerHour)
        .tenant_id(tenant)
        .hidden("203.0.113.7")
        .call()
        .await
        .unwrap()
}

/// The elements of every window of distinct elements in the database, as the database keeps
/// them.
async fn elements(pool: &PgPool) -> Vec<Vec<String>> {
    sqlx::query_scalar(
        "SELECT ARRAY(SELECT jsonb_array_elements_text(state->'elements')) \
         FROM zgm_limits.counters WHERE state ? 'elements' ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test]
async fn a_window_takes_at_most_so_many_different_elements(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let attempts = from_address(&limits, Uuid::new_v4()).await;
    let refused = Answer::Refused {
        until: noon() + HOUR,
    };

    // Three different accounts, a minute apart; one of them twice, which takes no room.
    for account in ["anna", "jan", "anna", "ewa"] {
        assert_eq!(
            attempts.hit_distinct(account).await.unwrap(),
            Answer::Allowed
        );
        clock.advance(MINUTE);
    }
    // A fourth account is refused until the end of the window, an hour after its first element.
    assert_eq!(attempts.hit_distinct("piotr").await.unwrap(), refused);
    assert_eq!(attempts.hit_distinct("ola").await.unwrap(), refused);
    // The three in the window still pass, any number of times.
    for account in ["anna", "jan", "ewa", "anna"] {
        assert_eq!(
            attempts.hit_distinct(account).await.unwrap(),
            Answer::Allowed
        );
    }
    // One row with three elements: neither the repeats nor the refusals added anything.
    assert_eq!(
        elements(&pool)
            .await
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        [3]
    );
}

#[sqlx::test]
async fn after_the_end_of_a_window_the_elements_start_again(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let attempts = from_address(&limits, Uuid::new_v4()).await;
    for account in ["anna", "jan", "ewa"] {
        let _counted = attempts.hit_distinct(account).await.unwrap();
    }

    // A second before the end a new element is still refused.
    clock.advance(HOUR - Duration::from_secs(1));
    assert!(matches!(
        attempts.hit_distinct("piotr").await.unwrap(),
        Answer::Refused { .. }
    ));

    // At the end a new window opens with that element alone, and ends an hour later.
    clock.advance(Duration::from_secs(1));
    for account in ["piotr", "ola", "ula"] {
        assert_eq!(
            attempts.hit_distinct(account).await.unwrap(),
            Answer::Allowed
        );
    }
    assert_eq!(
        attempts.hit_distinct("anna").await.unwrap(),
        Answer::Refused {
            until: noon() + HOUR + HOUR
        }
    );
    assert_eq!(
        elements(&pool)
            .await
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        [3]
    );
}

#[sqlx::test]
async fn an_element_reaches_the_database_only_as_a_hash(pool: PgPool) {
    let limits = limits(&pool);
    let attempts = from_address(&limits, Uuid::new_v4()).await;

    let _counted = attempts.hit_distinct("anna@firma.pl").await.unwrap();

    let stored = elements(&pool).await.remove(0).remove(0);
    assert!(!stored.contains("anna"));
    // The hash of the test: format byte, four bytes of key version, eight of hash, as text.
    assert_eq!(stored.len(), 26);
    assert!(stored.starts_with("0100000001"));
}

#[sqlx::test]
async fn every_key_has_a_window_of_elements_of_its_own(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();
    let first = from_address(&limits, tenant).await;
    for account in ["anna", "jan", "ewa"] {
        let _counted = first.hit_distinct(account).await.unwrap();
    }
    assert!(matches!(
        first.hit_distinct("piotr").await.unwrap(),
        Answer::Refused { .. }
    ));

    // Another address in the same tenant, and the same address in another tenant.
    let other_address = limits
        .attempts(TestLimit::ThreeDistinctPerHour)
        .tenant_id(tenant)
        .hidden("198.51.100.9")
        .call()
        .await
        .unwrap();
    let other_tenant = from_address(&limits, Uuid::new_v4()).await;

    assert_eq!(
        other_address.hit_distinct("piotr").await.unwrap(),
        Answer::Allowed
    );
    assert_eq!(
        other_tenant.hit_distinct("piotr").await.unwrap(),
        Answer::Allowed
    );
    // The same element has another hash in another tenant.
    let all = elements(&pool).await;
    assert_ne!(all[1], all[2]);
}

#[sqlx::test]
async fn elements_at_once_fill_the_window_and_not_more(
    pool_options: PgPoolOptions,
    connect_options: PgConnectOptions,
) {
    // Many connections, so that the attempts really run in the database at the same time.
    let pool = pool_options
        .max_connections(20)
        .connect_with(connect_options)
        .await
        .unwrap();
    let limits = limits(&pool);

    // Twenty rounds, each on a new key: 40 different elements at once, room for three.
    for _ in 0..20 {
        let tenant = Uuid::new_v4();
        let mut attempts = JoinSet::new();
        for number in 0..40 {
            let limits = limits.clone();
            attempts.spawn(async move {
                from_address(&limits, tenant)
                    .await
                    .hit_distinct(&format!("account-{number}"))
                    .await
                    .unwrap()
            });
        }
        let allowed = attempts
            .join_all()
            .await
            .into_iter()
            .filter(|answer| *answer == Answer::Allowed)
            .count();
        assert_eq!(allowed, 3);
    }
    // Every window stopped at three different elements.
    for window in elements(&pool).await {
        let mut distinct = window.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!((window.len(), distinct.len()), (3, 3));
    }
}

#[sqlx::test]
async fn the_same_element_at_once_takes_one_place(
    pool_options: PgPoolOptions,
    connect_options: PgConnectOptions,
) {
    let pool = pool_options
        .max_connections(20)
        .connect_with(connect_options)
        .await
        .unwrap();
    let limits = limits(&pool);

    for _ in 0..20 {
        let tenant = Uuid::new_v4();
        let mut attempts = JoinSet::new();
        for _ in 0..40 {
            let limits = limits.clone();
            attempts.spawn(async move {
                from_address(&limits, tenant)
                    .await
                    .hit_distinct("anna")
                    .await
                    .unwrap()
            });
        }
        // All forty pass: it is one account.
        assert!(
            attempts
                .join_all()
                .await
                .into_iter()
                .all(|answer| answer == Answer::Allowed)
        );
    }
    assert!(elements(&pool).await.iter().all(|window| window.len() == 1));
}

#[sqlx::test]
async fn an_element_is_known_after_a_change_of_the_secret_key(pool: PgPool) {
    let (limits, hash) = limits_with_hash(&pool);
    let tenant = Uuid::new_v4();
    // Two accounts before the change of the key.
    let before = from_address(&limits, tenant).await;
    let _counted = before.hit_distinct("anna").await.unwrap();
    let _counted = before.hit_distinct("jan").await.unwrap();

    // After it: version 2 is current, version 1 can still be read.
    hash.set_versions(&[2, 1]);
    let after = from_address(&limits, tenant).await;

    // The two are found under their old hashes and take no new room; a third one fits, and
    // a fourth does not.
    assert_eq!(after.hit_distinct("anna").await.unwrap(), Answer::Allowed);
    assert_eq!(after.hit_distinct("jan").await.unwrap(), Answer::Allowed);
    assert_eq!(after.hit_distinct("ewa").await.unwrap(), Answer::Allowed);
    assert!(matches!(
        after.hit_distinct("piotr").await.unwrap(),
        Answer::Refused { .. }
    ));
    let window = elements(&pool).await.remove(0);
    assert_eq!(window.len(), 3);
    // The new element was written with the current version of the key.
    assert!(window[2].starts_with("0100000002"));
}

#[sqlx::test]
async fn hit_and_hit_distinct_each_refuse_a_limit_of_the_other_kind(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();
    let of_elements = from_address(&limits, tenant).await;
    let of_attempts = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap();

    let hit_on_elements = of_elements.hit().await.unwrap_err();
    let distinct_on_attempts = of_attempts.hit_distinct("anna").await.unwrap_err();

    assert_eq!(
        hit_on_elements.to_string(),
        "the limit `three_distinct_per_hour` is not counted with `hit`: call `hit_distinct`"
    );
    assert_eq!(
        distinct_on_attempts.to_string(),
        "the limit `three_per_hour` is not counted with `hit_distinct`: call `hit`"
    );
    assert_eq!(counters(&pool).await, []);
}

#[sqlx::test]
async fn reset_forgets_the_whole_window_of_elements(pool: PgPool) {
    let limits = limits(&pool);
    let attempts = from_address(&limits, Uuid::new_v4()).await;
    for account in ["anna", "jan", "ewa"] {
        let _counted = attempts.hit_distinct(account).await.unwrap();
    }

    // The window was full, so the reset lifted a block.
    assert!(attempts.reset().await.unwrap());

    assert_eq!(elements(&pool).await, Vec::<Vec<String>>::new());
    assert_eq!(
        attempts.hit_distinct("piotr").await.unwrap(),
        Answer::Allowed
    );
}

#[sqlx::test]
async fn a_full_window_of_elements_is_on_the_list_of_blocked_keys_and_reported(pool: PgPool) {
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
    let tenant = Uuid::new_v4();
    let attempts = from_address(&limits, tenant).await;

    // Two elements: room left, nothing reported, nobody blocked.
    let _counted = attempts.hit_distinct("anna").await.unwrap();
    let _counted = attempts.hit_distinct("jan").await.unwrap();
    assert_eq!(audit.take(), []);
    // The third fills the window; a repeat reports nothing; a fourth is refused.
    let _counted = attempts.hit_distinct("ewa").await.unwrap();
    let _counted = attempts.hit_distinct("anna").await.unwrap();
    let _refused = attempts.hit_distinct("piotr").await.unwrap();

    let events = audit.take();
    assert!(matches!(
        events.as_slice(),
        [
            LimitsEvent::KeyBlocked { .. },
            LimitsEvent::AttemptRefused { .. }
        ]
    ));
    // No event holds an element.
    let printed = format!("{events:?}");
    assert!(!printed.contains("piotr") && !printed.contains("ewa"));

    let blocked = limits.get_blocked().tenant_id(tenant).call().await.unwrap();
    assert_eq!(blocked.items.len(), 1);
    assert_eq!(blocked.items[0].limit_name, "three_distinct_per_hour");
    assert_eq!(blocked.items[0].blocked_until, noon() + HOUR);
}

#[sqlx::test]
async fn a_tenant_sets_its_own_numbers_for_a_window_of_elements(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();

    // Two different elements per ten minutes instead of three an hour.
    let own = Rule::WindowOfDistinct {
        at_most: Count::new(2),
        per: Period::minutes(10),
    };
    assert!(
        limits
            .override_rule(TestLimit::ThreeDistinctPerHour)
            .tenant_id(tenant)
            .rule(own)
            .call()
            .await
            .unwrap()
    );

    let attempts = from_address(&limits, tenant).await;
    assert_eq!(
        attempts.hit_distinct("anna").await.unwrap(),
        Answer::Allowed
    );
    assert_eq!(attempts.hit_distinct("jan").await.unwrap(), Answer::Allowed);
    assert_eq!(
        attempts.hit_distinct("ewa").await.unwrap(),
        Answer::Refused {
            until: noon() + MINUTE * 10
        }
    );
    let rules = limits.get_rules().tenant_id(tenant).call().await.unwrap();
    let rule = rules
        .iter()
        .find(|rule| rule.limit_name == "three_distinct_per_hour")
        .unwrap();
    assert_eq!((rule.rule, rule.overridden), (own, true));
}

#[sqlx::test]
async fn a_tenant_cannot_change_what_a_limit_counts(pool: PgPool) {
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(TestHash::new())
        .audit(NoAudit)
        .limits(all_limits())
        .run_jobs_in_background(|_error| {})
        .build();
    let tenant = Uuid::new_v4();
    let of_attempts = Rule::Window {
        at_most: Count::new(3),
        per: Period::hours(1),
    };
    let of_elements = Rule::WindowOfDistinct {
        at_most: Count::new(3),
        per: Period::hours(1),
    };

    // A limit of attempts given a rule of elements, and the other way round.
    let first = limits
        .override_rule(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .rule(of_elements)
        .call()
        .await
        .unwrap_err();
    let second = limits
        .override_rule(TestLimit::ThreeDistinctPerHour)
        .tenant_id(tenant)
        .rule(of_attempts)
        .call()
        .await
        .unwrap_err();

    assert!(matches!(
        first,
        LimitsError::RuleOfAnotherKind {
            limit: "three_per_hour"
        }
    ));
    assert!(matches!(
        second,
        LimitsError::RuleOfAnotherKind {
            limit: "three_distinct_per_hour"
        }
    ));
    let rules = limits.get_rules().tenant_id(tenant).call().await.unwrap();
    assert!(rules.iter().all(|rule| !rule.overridden));
}
