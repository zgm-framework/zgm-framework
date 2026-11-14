//! The defaults from the code and the rules a tenant sets for itself.

mod common;

use common::{HOUR, MINUTE, TestLimit, counters, limits, limits_at_noon, noon, test_bounds};
use sqlx::PgPool;
use uuid::Uuid;
use zgm_limits::{Answer, Count, DefaultRule, Limits, LimitsError, Period, Rule, TenantRule};

/// One attempt of the whole tenant in `limit`.
async fn hit(limits: &Limits, limit: TestLimit, tenant: Uuid) -> Answer {
    limits
        .attempts(limit)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap()
}

/// A window of `at_most` attempts per `minutes` minutes.
fn window(at_most: u32, minutes: u32) -> Rule {
    Rule::Window {
        at_most: Count::new(at_most),
        per: Period::minutes(minutes),
    }
}

#[sqlx::test]
async fn the_defaults_are_the_rules_from_the_code_in_the_order_of_the_builder(pool: PgPool) {
    let limits = limits(&pool);

    let defaults = limits.get_defaults();

    assert_eq!(
        defaults,
        [
            DefaultRule {
                limit_name: "three_per_hour",
                rule: window(3, 60),
                bounds: Some(test_bounds()),
            },
            // No bounds in the code: a tenant cannot change this one.
            DefaultRule {
                limit_name: "other_three_per_hour",
                rule: window(3, 60),
                bounds: None,
            },
            DefaultRule {
                limit_name: "one_per_hour",
                rule: window(1, 60),
                bounds: Some(test_bounds()),
            },
            DefaultRule {
                limit_name: "bucket",
                rule: Rule::Bucket {
                    capacity: Count::new(20),
                    refill_every: Period::seconds(3),
                },
                bounds: None,
            },
            DefaultRule {
                limit_name: "three_distinct_per_hour",
                rule: Rule::WindowOfDistinct {
                    at_most: Count::new(3),
                    per: Period::hours(1),
                },
                bounds: Some(test_bounds()),
            },
        ]
    );
}

#[sqlx::test]
async fn a_tenant_with_no_rule_of_its_own_has_the_defaults(pool: PgPool) {
    let limits = limits(&pool);

    let rules = limits
        .get_rules()
        .tenant_id(Uuid::new_v4())
        .call()
        .await
        .unwrap();

    let expected: Vec<TenantRule> = limits
        .get_defaults()
        .iter()
        .map(|default| TenantRule {
            limit_name: default.limit_name,
            rule: default.rule,
            overridden: false,
            bounds: default.bounds,
        })
        .collect();
    assert_eq!(rules, expected);
}

#[sqlx::test]
async fn a_tenant_counts_by_its_own_rule_and_the_others_by_the_default(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let other_tenant = Uuid::new_v4();

    // The default is three an hour; this tenant wants one per ten minutes.
    let changed = limits
        .override_rule(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .rule(window(1, 10))
        .call()
        .await
        .unwrap();
    assert!(changed);

    // One attempt passes, and the refusal lasts ten minutes, not an hour.
    assert_eq!(
        hit(&limits, TestLimit::ThreePerHour, tenant).await,
        Answer::Allowed
    );
    assert_eq!(
        hit(&limits, TestLimit::ThreePerHour, tenant).await,
        Answer::Refused {
            until: noon() + MINUTE * 10
        }
    );
    // Another limit of the same tenant and the same limit of another tenant go by the default.
    for _ in 0..3 {
        assert_eq!(
            hit(&limits, TestLimit::OtherThreePerHour, tenant).await,
            Answer::Allowed
        );
        assert_eq!(
            hit(&limits, TestLimit::ThreePerHour, other_tenant).await,
            Answer::Allowed
        );
    }
    assert_eq!(
        hit(&limits, TestLimit::ThreePerHour, other_tenant).await,
        Answer::Refused {
            until: noon() + HOUR
        }
    );

    // The list of the tenant's rules shows which one is its own.
    let rules = limits.get_rules().tenant_id(tenant).call().await.unwrap();
    assert_eq!(
        rules[0],
        TenantRule {
            limit_name: "three_per_hour",
            rule: window(1, 10),
            overridden: true,
            bounds: Some(test_bounds()),
        }
    );
    assert!(rules[1..].iter().all(|rule| !rule.overridden));
}

#[sqlx::test]
async fn an_open_window_keeps_the_rule_it_was_opened_with(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let with_open_window = Uuid::new_v4();
    let blocked = Uuid::new_v4();
    let hit_of = async |account: Uuid| {
        limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .subject_id(account)
            .call()
            .await
            .unwrap()
            .hit()
            .await
            .unwrap()
    };
    // Under the default, three an hour: one account has one attempt, another is blocked.
    let _counted = hit_of(with_open_window).await;
    for _ in 0..3 {
        let _counted = hit_of(blocked).await;
    }

    // The tenant makes the limit stricter: one attempt per ten minutes.
    limits
        .override_rule(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .rule(window(1, 10))
        .call()
        .await
        .unwrap();

    // Nothing was deleted and no block was lifted.
    assert_eq!(counters(&pool).await.len(), 2);
    let on_the_list = limits.get_blocked().tenant_id(tenant).call().await.unwrap();
    assert_eq!(on_the_list.items.len(), 1);
    assert_eq!(on_the_list.items[0].subject_id, blocked);
    // The open window still lets three through and still ends at 13:00.
    assert_eq!(hit_of(with_open_window).await, Answer::Allowed);
    assert_eq!(hit_of(with_open_window).await, Answer::Allowed);
    assert_eq!(
        hit_of(with_open_window).await,
        Answer::Refused {
            until: noon() + HOUR
        }
    );
    // An account with no window opens one under the new rule: one attempt, ten minutes.
    let new_account = Uuid::new_v4();
    assert_eq!(hit_of(new_account).await, Answer::Allowed);
    assert_eq!(
        hit_of(new_account).await,
        Answer::Refused {
            until: noon() + MINUTE * 10
        }
    );

    // When the old window is over, the next one of that account is under the new rule too.
    clock.advance(HOUR);
    assert_eq!(hit_of(with_open_window).await, Answer::Allowed);
    assert_eq!(
        hit_of(with_open_window).await,
        Answer::Refused {
            until: noon() + HOUR + MINUTE * 10
        }
    );
}

#[sqlx::test]
async fn a_looser_rule_does_not_unblock_who_is_blocked_but_the_list_still_can(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let _counted = hit(&limits, TestLimit::OnePerHour, tenant).await;

    // From one an hour to five an hour.
    limits
        .override_rule(TestLimit::OnePerHour)
        .tenant_id(tenant)
        .rule(window(5, 60))
        .call()
        .await
        .unwrap();

    // The window that was full stays full, and the list of blocked keys agrees with it.
    assert!(matches!(
        hit(&limits, TestLimit::OnePerHour, tenant).await,
        Answer::Refused { .. }
    ));
    let on_the_list = limits.get_blocked().tenant_id(tenant).call().await.unwrap();
    assert_eq!(on_the_list.items.len(), 1);

    // Unblocked from the list, the key opens a window under the new rule: five pass.
    assert!(
        limits
            .unblock(tenant, on_the_list.items[0].id)
            .await
            .unwrap()
    );
    for _ in 0..5 {
        assert_eq!(
            hit(&limits, TestLimit::OnePerHour, tenant).await,
            Answer::Allowed
        );
    }
    assert!(matches!(
        hit(&limits, TestLimit::OnePerHour, tenant).await,
        Answer::Refused { .. }
    ));
}

#[sqlx::test]
async fn the_same_rule_set_again_changes_nothing(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();
    let set = async || {
        limits
            .override_rule(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .rule(window(5, 60))
            .call()
            .await
            .unwrap()
    };
    assert!(set().await);
    let _counted = hit(&limits, TestLimit::ThreePerHour, tenant).await;

    let changed = set().await;

    assert!(!changed);
    assert_eq!(counters(&pool).await.len(), 1);
}

#[sqlx::test]
async fn restoring_the_default_brings_the_rule_from_the_code_back(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    limits
        .override_rule(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .rule(window(1, 10))
        .call()
        .await
        .unwrap();
    // A window opened under the tenant's own rule, full after one attempt.
    let _counted = hit(&limits, TestLimit::ThreePerHour, tenant).await;

    let had_own_rule = limits
        .restore_default(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap();

    assert!(had_own_rule);
    let rules = limits.get_rules().tenant_id(tenant).call().await.unwrap();
    assert!(rules.iter().all(|rule| !rule.overridden));
    // That window stays as it was opened: refused until 12:10.
    assert_eq!(
        hit(&limits, TestLimit::ThreePerHour, tenant).await,
        Answer::Refused {
            until: noon() + MINUTE * 10
        }
    );
    // After its end the default counts: three an hour.
    clock.advance(MINUTE * 10);
    for _ in 0..3 {
        assert_eq!(
            hit(&limits, TestLimit::ThreePerHour, tenant).await,
            Answer::Allowed
        );
    }
    assert_eq!(
        hit(&limits, TestLimit::ThreePerHour, tenant).await,
        Answer::Refused {
            until: noon() + MINUTE * 10 + HOUR
        }
    );

    // Restoring once more: there is nothing to restore.
    let again = limits
        .restore_default(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap();
    assert!(!again);
}

#[sqlx::test]
async fn a_rule_that_cannot_be_counted_and_a_limit_not_in_the_code_are_refused(pool: PgPool) {
    let tenant = Uuid::new_v4();
    // A bucket cannot be counted yet, so a tenant cannot pick it.
    let all = limits(&pool);
    let bucket = all
        .override_rule(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .rule(Rule::Bucket {
            capacity: Count::new(20),
            refill_every: Period::seconds(3),
        })
        .call()
        .await
        .unwrap_err();
    assert!(matches!(bucket, LimitsError::BucketNotImplemented));

    // A module that knows only one limit refuses a rule for another.
    let one_limit = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(common::TestHash::new())
        .audit(common::NoAudit)
        .limits(&[TestLimit::ThreePerHour])
        .run_jobs_in_background(|_error| {})
        .build();
    let unknown = one_limit
        .override_rule(TestLimit::OnePerHour)
        .tenant_id(tenant)
        .rule(window(5, 60))
        .call()
        .await
        .unwrap_err();
    assert!(matches!(
        unknown,
        LimitsError::UnknownLimit {
            limit: "one_per_hour"
        }
    ));

    // Neither wrote a rule.
    let rules = all.get_rules().tenant_id(tenant).call().await.unwrap();
    assert!(rules.iter().all(|rule| !rule.overridden));
}

#[sqlx::test]
async fn a_window_of_distinct_elements_keeps_the_rule_it_was_opened_with_too(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let from_address = limits
        .attempts(TestLimit::ThreeDistinctPerHour)
        .tenant_id(tenant)
        .hidden("203.0.113.7")
        .call()
        .await
        .unwrap();
    // A window opened under the default, three different elements an hour, with one in it.
    assert_eq!(
        from_address.hit_distinct("anna").await.unwrap(),
        Answer::Allowed
    );

    // The tenant sets one element per ten minutes.
    limits
        .override_rule(TestLimit::ThreeDistinctPerHour)
        .tenant_id(tenant)
        .rule(Rule::WindowOfDistinct {
            at_most: Count::new(1),
            per: Period::minutes(10),
        })
        .call()
        .await
        .unwrap();

    // The open window still takes three and ends at 13:00.
    assert_eq!(
        from_address.hit_distinct("jan").await.unwrap(),
        Answer::Allowed
    );
    assert_eq!(
        from_address.hit_distinct("ewa").await.unwrap(),
        Answer::Allowed
    );
    assert_eq!(
        from_address.hit_distinct("piotr").await.unwrap(),
        Answer::Refused {
            until: noon() + HOUR
        }
    );
}

#[sqlx::test]
async fn a_rule_outside_the_bounds_of_the_limit_is_refused(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();
    // The bounds of `three_per_hour`: 1 to 10 attempts per one minute to one day.
    let set = async |rule: Rule| {
        limits
            .override_rule(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .rule(rule)
            .call()
            .await
    };

    let too_many = set(window(11, 60)).await.unwrap_err();
    let too_short = set(Rule::Window {
        at_most: Count::new(3),
        per: Period::seconds(59),
    })
    .await
    .unwrap_err();
    let too_long = set(Rule::Window {
        at_most: Count::new(3),
        per: Period::seconds(86_401),
    })
    .await
    .unwrap_err();

    assert_eq!(
        too_many.to_string(),
        "`at_most` of 11 is outside what the limit `three_per_hour` allows, 1 to 10"
    );
    assert_eq!(
        too_short.to_string(),
        "`per_seconds` of 59 is outside what the limit `three_per_hour` allows, 60 to 86400"
    );
    assert!(matches!(too_long, LimitsError::RuleOutOfBounds { .. }));
    // None of them wrote a rule; the edges themselves are allowed.
    let rules = limits.get_rules().tenant_id(tenant).call().await.unwrap();
    assert!(rules.iter().all(|rule| !rule.overridden));
    assert!(set(window(10, 1)).await.unwrap());
    assert!(set(window(1, 24 * 60)).await.unwrap());
}

#[sqlx::test]
async fn a_limit_with_no_bounds_cannot_be_changed_by_a_tenant(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();

    let error = limits
        .override_rule(TestLimit::OtherThreePerHour)
        .tenant_id(tenant)
        .rule(window(3, 60))
        .call()
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        LimitsError::RuleCannotBeChanged {
            limit: "other_three_per_hour"
        }
    ));
}

#[sqlx::test]
async fn a_row_from_before_narrower_bounds_counts_within_the_bounds_of_today(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    // Rows as an older version of the product could have left them: one far above the bounds
    // of today (at most 10, at most a day), one for a limit that has no bounds any more.
    sqlx::query(
        "INSERT INTO zgm_limits.tenant_rules (tenant_id, limit_name, rule) VALUES \
             ($1, 'one_per_hour', '{\"kind\": \"window\", \"at_most\": 4000000000, \
                                    \"per_seconds\": 4000000000}'), \
             ($1, 'other_three_per_hour', '{\"kind\": \"window\", \"at_most\": 1000, \
                                            \"per_seconds\": 60}')",
    )
    .bind(tenant)
    .execute(&pool)
    .await
    .unwrap();

    // The first counts as the highest the bounds allow: ten attempts, and a window of a day.
    for _ in 0..10 {
        assert_eq!(
            hit(&limits, TestLimit::OnePerHour, tenant).await,
            Answer::Allowed
        );
    }
    assert_eq!(
        hit(&limits, TestLimit::OnePerHour, tenant).await,
        Answer::Refused {
            until: noon() + HOUR * 24
        }
    );
    // The second counts as the default, three an hour: with no bounds nothing may change.
    for _ in 0..3 {
        assert_eq!(
            hit(&limits, TestLimit::OtherThreePerHour, tenant).await,
            Answer::Allowed
        );
    }
    assert_eq!(
        hit(&limits, TestLimit::OtherThreePerHour, tenant).await,
        Answer::Refused {
            until: noon() + HOUR
        }
    );

    // The list of the tenant's rules says the same as the counting.
    let rules = limits.get_rules().tenant_id(tenant).call().await.unwrap();
    assert_eq!(rules[1].rule, window(3, 60));
    assert_eq!(
        rules[2].rule,
        Rule::Window {
            at_most: Count::new(10),
            per: Period::days(1),
        }
    );
}

#[sqlx::test]
async fn a_row_with_a_way_of_counting_this_version_does_not_know_is_passed_over(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    // As a newer version of the module could have left it, before a step back to this one.
    sqlx::query(
        "INSERT INTO zgm_limits.tenant_rules (tenant_id, limit_name, rule) \
         VALUES ($1, 'three_per_hour', '{\"kind\": \"something_new\", \"at_most\": 1, \
                                         \"per_seconds\": 60}')",
    )
    .bind(tenant)
    .execute(&pool)
    .await
    .unwrap();

    // The counting goes by the default, three an hour, and so does the list of rules.
    for _ in 0..3 {
        assert_eq!(
            hit(&limits, TestLimit::ThreePerHour, tenant).await,
            Answer::Allowed
        );
    }
    assert_eq!(
        hit(&limits, TestLimit::ThreePerHour, tenant).await,
        Answer::Refused {
            until: noon() + HOUR
        }
    );
    let rules = limits.get_rules().tenant_id(tenant).call().await.unwrap();
    assert_eq!(rules[0].rule, window(3, 60));
    assert!(!rules[0].overridden);
}

#[sqlx::test]
async fn a_rule_changed_while_attempts_are_running_leaves_no_error_and_the_new_rule_in_force(
    pool_options: sqlx::postgres::PgPoolOptions,
    connect_options: sqlx::postgres::PgConnectOptions,
) {
    // Many connections, so that the attempts and the changes really run at the same time.
    let pool = pool_options
        .max_connections(20)
        .connect_with(connect_options)
        .await
        .unwrap();
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();

    // 200 attempts on 20 accounts, and in the middle of them the rule changes five times.
    let mut running = tokio::task::JoinSet::new();
    let accounts: Vec<Uuid> = (0..20).map(|_| Uuid::new_v4()).collect();
    for number in 0..200 {
        let limits = limits.clone();
        let account = accounts[number % accounts.len()];
        running.spawn(async move {
            limits
                .attempts(TestLimit::ThreePerHour)
                .tenant_id(tenant)
                .subject_id(account)
                .call()
                .await
                .unwrap()
                .hit()
                .await
                .map(|_answer| ())
        });
    }
    for at_most in [1, 5, 2, 10, 1] {
        let limits = limits.clone();
        running.spawn(async move {
            limits
                .override_rule(TestLimit::ThreePerHour)
                .tenant_id(tenant)
                .rule(window(at_most, 60))
                .call()
                .await
                .map(|_changed| ())
        });
    }
    // Neither an attempt nor a change failed, whatever order the database ran them in.
    for outcome in running.join_all().await {
        outcome.unwrap();
    }

    // One more change, with nothing running. Accounts that never tried open their window
    // under it: two attempts pass and the third is refused.
    limits
        .override_rule(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .rule(window(2, 60))
        .call()
        .await
        .unwrap();
    for _ in 0..20 {
        let attempts = limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .subject_id(Uuid::new_v4())
            .call()
            .await
            .unwrap();
        assert_eq!(attempts.hit().await.unwrap(), Answer::Allowed);
        assert_eq!(attempts.hit().await.unwrap(), Answer::Allowed);
        assert!(matches!(
            attempts.hit().await.unwrap(),
            Answer::Refused { .. }
        ));
    }
    // No window written during the changes holds more attempts than the limit it kept: every
    // one of them was opened under one of the rules that were in force, and counts by it.
    let over_their_own_limit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM zgm_limits.counters \
         WHERE (state->>'count')::bigint > (state->>'at_most')::bigint",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(over_their_own_limit, 0);
}
