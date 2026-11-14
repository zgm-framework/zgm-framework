//! `Attempts::reset` on a real database, and a whole sign-in built from `hit` and `reset`.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use common::{HOUR, TestLimit, counters, limits, limits_at_noon, limits_with_hash};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::task::JoinSet;
use uuid::Uuid;
use zgm_limits::{Answer, KeyedHash, KeyedHashError, Limits};

#[sqlx::test]
async fn reset_deletes_the_counter_so_the_key_starts_from_nothing(pool: PgPool) {
    let (limits, _clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let account = Uuid::new_v4();
    let attempts = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .subject_id(account)
        .call()
        .await
        .unwrap();
    // Another account of the same tenant, with a counter of its own.
    let other = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .subject_id(Uuid::new_v4())
        .call()
        .await
        .unwrap();
    let _counted = other.hit().await.unwrap();

    // Nothing to forget yet.
    assert!(!attempts.reset().await.unwrap());

    // Two attempts and then a success: there was no block to lift.
    let _counted = attempts.hit().await.unwrap();
    let _counted = attempts.hit().await.unwrap();
    assert!(!attempts.reset().await.unwrap());
    // Only the counter of the other account is left.
    assert_eq!(counters(&pool).await, [(Vec::new(), 1)]);

    // The count starts from nothing: three attempts pass again and the fourth is refused.
    for _ in 0..3 {
        assert_eq!(attempts.hit().await.unwrap(), Answer::Allowed);
    }
    assert!(matches!(
        attempts.hit().await.unwrap(),
        Answer::Refused { .. }
    ));

    // Now there is a block, and reset says that it lifted it.
    assert!(attempts.reset().await.unwrap());
    assert_eq!(attempts.hit().await.unwrap(), Answer::Allowed);
}

#[sqlx::test]
async fn reset_of_a_block_that_is_over_lifts_nothing(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let attempts = limits
        .attempts(TestLimit::OnePerHour)
        .tenant_id(Uuid::new_v4())
        .call()
        .await
        .unwrap();
    let _counted = attempts.hit().await.unwrap();

    clock.advance(HOUR);

    // The row is deleted, but its block had already ended on its own.
    assert!(!attempts.reset().await.unwrap());
    assert_eq!(counters(&pool).await, []);
}

#[sqlx::test]
async fn reset_deletes_the_counters_of_every_version_of_the_secret_key(pool: PgPool) {
    let (limits, hash) = limits_with_hash(&pool);
    let tenant = Uuid::new_v4();
    let attempts_of_anna = async || {
        limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .hidden("anna@firma.pl")
            .call()
            .await
            .unwrap()
    };
    // A full window under version 1 of the key.
    let before_the_change = attempts_of_anna().await;
    for _ in 0..3 {
        let _counted = before_the_change.hit().await.unwrap();
    }
    // A server that reads only version 2 opens a second row for the same e-mail.
    hash.set_versions(&[2]);
    let _counted = attempts_of_anna().await.hit().await.unwrap();
    assert_eq!(counters(&pool).await.len(), 2);

    // With both versions readable, reset deletes both rows; the one of version 1 was a block.
    hash.set_versions(&[2, 1]);
    assert!(attempts_of_anna().await.reset().await.unwrap());
    assert_eq!(counters(&pool).await, []);
}

/// A keyed hash that counts how many times it was asked.
#[derive(Clone, Default)]
struct CountingHash(Arc<AtomicUsize>);

#[async_trait]
impl KeyedHash for CountingHash {
    async fn hashes(&self, data: &[u8]) -> Result<Vec<Vec<u8>>, KeyedHashError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        // Any short value that depends on the data will do.
        Ok(vec![vec![1, data.len() as u8]])
    }
}

#[sqlx::test]
async fn the_hash_is_computed_once_for_all_calls_on_the_attempts(pool: PgPool) {
    let hash = CountingHash::default();
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(hash.clone())
        .audit(common::NoAudit)
        .limits(common::all_limits())
        .run_jobs_in_background(|_error| {})
        .build();

    // A whole sign-in: count the attempt, then forget it.
    let attempts = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(Uuid::new_v4())
        .hidden("anna@firma.pl")
        .call()
        .await
        .unwrap();
    let _counted = attempts.hit().await.unwrap();
    attempts.reset().await.unwrap();
    assert_eq!(hash.0.load(Ordering::SeqCst), 1);

    // A key with no hidden part does not ask the hash at all.
    let whole_tenant = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(Uuid::new_v4())
        .call()
        .await
        .unwrap();
    let _counted = whole_tenant.hit().await.unwrap();
    assert_eq!(hash.0.load(Ordering::SeqCst), 1);
}

#[sqlx::test]
async fn sign_ins_sent_at_once_get_no_more_passwords_checked_than_the_limit_lets(
    pool_options: PgPoolOptions,
    connect_options: PgConnectOptions,
) {
    // Many connections, so that the sign-ins really run in the database at the same time.
    let pool = pool_options
        .max_connections(20)
        .connect_with(connect_options)
        .await
        .unwrap();
    let limits = limits(&pool);

    // Ten rounds, each on a new account: 60 sign-ins at once, and the last of them has the
    // right password.
    for _ in 0..10 {
        let tenant = Uuid::new_v4();
        let account = Uuid::new_v4();
        let passwords_checked = Arc::new(AtomicUsize::new(0));
        let signed_in = Arc::new(AtomicUsize::new(0));

        let mut sign_ins = JoinSet::new();
        for number in 0..60 {
            let limits = limits.clone();
            let passwords_checked = passwords_checked.clone();
            let signed_in = signed_in.clone();
            sign_ins.spawn(async move {
                let attempts = limits
                    .attempts(TestLimit::ThreePerHour)
                    .tenant_id(tenant)
                    .subject_id(account)
                    .call()
                    .await
                    .unwrap();
                // Count first; only an attempt that passed gets its password checked.
                if attempts.hit().await.unwrap() != Answer::Allowed {
                    return;
                }
                passwords_checked.fetch_add(1, Ordering::SeqCst);
                // The check itself takes a while, as a password hash does.
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                let password_is_right = number == 59;
                if password_is_right {
                    signed_in.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
        sign_ins.join_all().await;

        // Three passwords were checked, not sixty, whichever sign-ins came first.
        assert_eq!(passwords_checked.load(Ordering::SeqCst), 3);
        assert!(signed_in.load(Ordering::SeqCst) <= 1);
    }
}
