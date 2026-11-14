//! The loop of the cleanup on a real database. `build()` runs the first pass at once, so a
//! test moves its clock and builds the module again to get a pass at the moment it wants.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{HOUR, TestClock, TestHash, TestLimit, noon};
use sqlx::PgPool;
use uuid::Uuid;
use zgm_limits::Limits;

/// The module, built at this moment: its loop starts its first pass at once.
fn built_now(pool: &PgPool, clock: &TestClock) -> Limits {
    Limits::builder()
        .pool(pool.clone())
        .keyed_hash(TestHash::new())
        .clock(clock.clone())
        .audit(common::NoAudit)
        .limits(common::all_limits())
        // A failed pass shows up as counters that are not deleted, which the tests wait for.
        .run_jobs_in_background(|error| eprintln!("the cleanup failed: {}", error.error))
        .build()
}

/// The names of the limits that still have a counter, in order.
async fn limits_with_counters(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT limit_name FROM zgm_limits.counters ORDER BY limit_name")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// Waits, for at most five seconds, until the table has `expected` counters.
async fn wait_for_counters(pool: &PgPool, expected: i64) {
    for _ in 0..500 {
        let counters: i64 = sqlx::query_scalar("SELECT count(*) FROM zgm_limits.counters")
            .fetch_one(pool)
            .await
            .unwrap();
        if counters == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the table did not get to {expected} counters");
}

#[sqlx::test]
async fn the_loop_deletes_counters_an_hour_after_their_end_and_leaves_the_others(pool: PgPool) {
    let clock = TestClock::at_noon();
    let limits = built_now(&pool, &clock);
    let tenant = Uuid::new_v4();
    // A window that ends at 13:00 and one that ends at 13:30.
    let _counted = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    clock.advance(HOUR / 2);
    let _counted = limits
        .attempts(TestLimit::OtherThreePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();

    // A pass at 13:59:59. The first window ended almost an hour ago, but a counter stays for
    // a whole hour after its end, so nothing goes.
    clock.advance(HOUR + HOUR / 2 - Duration::from_secs(1));
    let _second = built_now(&pool, &clock);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        limits_with_counters(&pool).await,
        ["other_three_per_hour", "three_per_hour"]
    );

    // A pass at 14:00: the first window ended an hour ago, the second only half an hour ago.
    clock.advance(Duration::from_secs(1));
    let _third = built_now(&pool, &clock);
    wait_for_counters(&pool, 1).await;
    assert_eq!(limits_with_counters(&pool).await, ["other_three_per_hour"]);
}

#[sqlx::test]
async fn one_pass_deletes_more_than_one_batch(pool: PgPool) {
    // 250 counters that ended two hours before noon: two full batches and a half.
    sqlx::query(
        "INSERT INTO zgm_limits.counters \
             (limit_name, tenant_id, subject_id, key_hash, state, expires_at) \
         SELECT 'three_per_hour', uuidv7(), '00000000-0000-0000-0000-000000000000', '', \
                '{\"count\": 1}', $1 \
         FROM generate_series(1, 250)",
    )
    .bind(noon() - HOUR - HOUR)
    .execute(&pool)
    .await
    .unwrap();

    let _limits = built_now(&pool, &TestClock::at_noon());

    wait_for_counters(&pool, 0).await;
}

#[tokio::test(start_paused = true)]
async fn a_panic_in_the_function_for_errors_does_not_end_the_loop() {
    // A pool that is closed: every pass of the cleanup fails at once, with no database at all.
    let pool = PgPool::connect_lazy("postgres://nobody@localhost/nothing").unwrap();
    pool.close().await;
    let reported = Arc::new(AtomicUsize::new(0));
    let reported_in_loop = reported.clone();

    let _limits = Limits::builder()
        .pool(pool)
        .keyed_hash(TestHash::new())
        .audit(common::NoAudit)
        .limits(common::all_limits())
        .run_jobs_in_background(move |_error| {
            reported_in_loop.fetch_add(1, Ordering::SeqCst);
            panic!("the function for errors itself fails");
        })
        .build();

    // The first pass runs at once, then one every two minutes. The clock of the test is
    // paused, so three minutes pass in an instant, three times.
    for passes in 1..=4 {
        for _ in 0..100 {
            if reported.load(Ordering::SeqCst) == passes {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(reported.load(Ordering::SeqCst), passes);
        tokio::time::advance(Duration::from_mins(3)).await;
    }
}
