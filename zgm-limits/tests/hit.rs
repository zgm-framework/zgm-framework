//! `Limits::hit` on a real database. `#[sqlx::test]` gives every test a new, empty database
//! with the migrations applied; it needs a PostgreSQL 18 server in `DATABASE_URL`.

mod common;

use std::time::Duration;

use async_trait::async_trait;
use common::{
    HOUR, MINUTE, TestLimit, counters, key_version, limits, limits_at_noon, limits_with_hash, noon,
    refused_until,
};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use tokio::task::JoinSet;
use uuid::Uuid;
use zgm_limits::{Answer, KeyedHash, KeyedHashError, Limits, LimitsError, MAX_HASH_BYTES};

#[sqlx::test]
async fn a_window_lets_through_at_most_and_then_refuses(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let hit = async || {
        limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .call()
            .await
            .unwrap()
            .hit()
            .await
    };

    // Three attempts, a minute apart: 12:00, 12:01, 12:02.
    for _ in 0..3 {
        assert_eq!(hit().await.unwrap(), Answer::Allowed);
        clock.advance(MINUTE);
    }

    // The window ends an hour after its first attempt, not after the last one.
    let refused = hit().await.unwrap();
    assert_eq!(
        refused,
        Answer::Refused {
            until: noon() + HOUR
        }
    );
}

#[sqlx::test]
async fn a_refusal_changes_nothing(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();
    for _ in 0..3 {
        let _counted = limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .call()
            .await
            .unwrap()
            .hit()
            .await
            .unwrap();
    }

    let first = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    let second = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();

    // The end of the window does not move, and the count stays at three.
    assert_eq!(refused_until(first), refused_until(second));
    assert_eq!(counters(&pool).await, [(Vec::new(), 3)]);
}

#[sqlx::test]
async fn after_the_end_of_a_window_the_count_starts_again(pool: PgPool) {
    let (limits, clock) = limits_at_noon(&pool);
    let tenant = Uuid::new_v4();
    let hit = async || {
        limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .call()
            .await
            .unwrap()
            .hit()
            .await
    };
    for _ in 0..3 {
        let _counted = hit().await.unwrap();
    }

    // A second before the end the window is still full.
    clock.advance(HOUR - Duration::from_secs(1));
    assert_eq!(
        hit().await.unwrap(),
        Answer::Refused {
            until: noon() + HOUR
        }
    );

    // At the end, 13:00, a new window opens: three attempts pass again, and it ends at 14:00.
    clock.advance(Duration::from_secs(1));
    for _ in 0..3 {
        assert_eq!(hit().await.unwrap(), Answer::Allowed);
    }
    assert_eq!(
        hit().await.unwrap(),
        Answer::Refused {
            until: noon() + HOUR + HOUR
        }
    );
    // Still one row: the new window took the place of the old one.
    assert_eq!(counters(&pool).await, [(Vec::new(), 3)]);
}

#[sqlx::test]
async fn every_limit_tenant_account_and_hidden_part_has_a_counter_of_its_own(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();
    let account = Uuid::new_v4();
    let hidden = "anna@firma.pl";
    for _ in 0..3 {
        let _counted = limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .subject_id(account)
            .hidden(hidden)
            .call()
            .await
            .unwrap()
            .hit()
            .await
            .unwrap();
    }
    let full = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .subject_id(account)
        .hidden(hidden)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    assert!(matches!(full, Answer::Refused { .. }));

    // Change one part at a time: each time it is another counter, with room in it.
    let other_limit = limits
        .attempts(TestLimit::OtherThreePerHour)
        .tenant_id(tenant)
        .subject_id(account)
        .hidden(hidden)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    let other_tenant = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(Uuid::new_v4())
        .subject_id(account)
        .hidden(hidden)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    let other_account = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .subject_id(Uuid::new_v4())
        .hidden(hidden)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    let other_hidden_part = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .subject_id(account)
        .hidden("jan@firma.pl")
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();

    assert_eq!(other_limit, Answer::Allowed);
    assert_eq!(other_tenant, Answer::Allowed);
    assert_eq!(other_account, Answer::Allowed);
    assert_eq!(other_hidden_part, Answer::Allowed);
}

/// The version of the key and the count of every counter, in the order of the versions.
async fn counters_by_key_version(pool: &PgPool) -> Vec<(Option<u32>, i64)> {
    counters(pool)
        .await
        .into_iter()
        .map(|(hash, count)| (key_version(&hash), count))
        .collect()
}

#[sqlx::test]
async fn the_hidden_part_reaches_the_database_only_as_a_hash(pool: PgPool) {
    let limits = limits(&pool);

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

    let (hash, count) = counters(&pool).await.remove(0);
    assert_eq!(count, 1);
    assert!(!hash.is_empty());
    assert!(!String::from_utf8_lossy(&hash).contains("anna"));
}

#[sqlx::test]
async fn hidden_makes_the_text_uniform_so_another_spelling_is_the_same_key(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();

    // Three spellings of one e-mail: upper case, white space at the ends and in the middle.
    for spelling in ["anna@firma.pl", "  Anna@Firma.PL\t", "anna @firma.pl\n"] {
        let answer = limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .hidden(spelling)
            .call()
            .await
            .unwrap()
            .hit()
            .await
            .unwrap();
        assert_eq!(answer, Answer::Allowed);
    }
    let fourth = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .hidden("ANNA@FIRMA.PL")
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();

    // All of them counted on one counter, so the fourth is refused.
    assert!(matches!(fourth, Answer::Refused { .. }));
    assert_eq!(counters(&pool).await.len(), 1);
}

#[sqlx::test]
async fn hidden_drops_invisible_characters_so_they_give_no_counter_of_their_own(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();
    // One e-mail with a character nobody sees pasted into it, a different one each time:
    // zero-width space, byte order mark, soft hyphen, word joiner, a direction override, a
    // variation selector, a control character and a tag character.
    let spellings = [
        "anna@firma.pl",
        "an\u{200B}na@firma.pl",
        "\u{FEFF}anna@firma.pl",
        "anna@fir\u{00AD}ma.pl",
        "anna\u{2060}@firma.pl",
        "anna@firma\u{202E}.pl",
        "anna@firma.pl\u{FE0F}",
        "anna@\u{0007}firma.pl",
        "a\u{E0061}nna@firma.pl",
    ];

    for spelling in spellings {
        let _counted = limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .hidden(spelling)
            .call()
            .await
            .unwrap()
            .hit()
            .await
            .unwrap();
    }

    // Nine spellings, one counter: the first three passed, the rest were refused.
    assert_eq!(
        counters(&pool)
            .await
            .into_iter()
            .map(|(_, count)| count)
            .collect::<Vec<_>>(),
        [3]
    );
}

#[sqlx::test]
async fn hidden_exact_takes_the_text_as_it_is(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();
    let hit_exact = async |text: &str| {
        limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .hidden_exact(text)
            .call()
            .await
            .unwrap()
            .hit()
            .await
            .unwrap()
    };

    // Another case and a space are other keys.
    let _counted = hit_exact("Key-A1").await;
    let _counted = hit_exact("key-a1").await;
    let _counted = hit_exact("key-a1 ").await;
    assert_eq!(counters(&pool).await.len(), 3);

    // A text that is already uniform is the same key through `hidden` and `hidden_exact`.
    let _counted = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .hidden("KEY-A1")
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    let mut counts: Vec<i64> = counters(&pool)
        .await
        .into_iter()
        .map(|(_, count)| count)
        .collect();
    counts.sort();
    assert_eq!(counts, [1, 1, 2]);
}

#[sqlx::test]
async fn the_nil_uuid_as_an_account_is_an_error_and_counts_nothing(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();

    // E.g. an account that was not found, turned into a default value.
    let error = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .subject_id(Uuid::nil())
        .call()
        .await
        .unwrap_err();

    assert!(matches!(error, LimitsError::NilSubject));
    assert_eq!(counters(&pool).await, []);
    // No account at all is the honest way to ask for the whole tenant.
    let whole_tenant = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(tenant)
        .maybe_subject_id(None)
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    assert_eq!(whole_tenant, Answer::Allowed);
}

#[sqlx::test]
async fn hidden_together_with_hidden_exact_is_an_error_and_counts_nothing(pool: PgPool) {
    let limits = limits(&pool);

    let error = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(Uuid::new_v4())
        .hidden("anna@firma.pl")
        .hidden_exact("anna@firma.pl")
        .call()
        .await
        .unwrap_err();

    assert!(matches!(error, LimitsError::HiddenAndHiddenExact));
    assert_eq!(counters(&pool).await, []);
}

#[sqlx::test]
async fn the_same_hidden_part_has_another_hash_in_every_limit_and_tenant(pool: PgPool) {
    let limits = limits(&pool);
    let tenant = Uuid::new_v4();
    let hit = async |limit: TestLimit, tenant: Uuid| {
        let _counted = limits
            .attempts(limit)
            .tenant_id(tenant)
            .hidden("anna@firma.pl")
            .call()
            .await
            .unwrap()
            .hit()
            .await
            .unwrap();
    };

    hit(TestLimit::ThreePerHour, tenant).await;
    hit(TestLimit::OtherThreePerHour, tenant).await;
    hit(TestLimit::ThreePerHour, Uuid::new_v4()).await;

    // Three rows, and no two of them share a hash: a copy of the database does not show that
    // they are about the same e-mail.
    let mut hashes: Vec<Vec<u8>> = counters(&pool)
        .await
        .into_iter()
        .map(|(hash, _)| hash)
        .collect();
    hashes.dedup();
    assert_eq!(hashes.len(), 3);
}

#[sqlx::test]
async fn a_change_of_the_secret_key_does_not_start_the_count_again(pool: PgPool) {
    let (limits, hash) = limits_with_hash(&pool);
    let tenant = Uuid::new_v4();
    let hit = async || {
        limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .hidden("anna@firma.pl")
            .call()
            .await
            .unwrap()
            .hit()
            .await
    };

    // Two attempts before the change of the key, with the only version there is.
    let _counted = hit().await.unwrap();
    let _counted = hit().await.unwrap();

    // After the change: version 2 is current, version 1 can still be read.
    hash.set_versions(&[2, 1]);
    let third = hit().await.unwrap();
    let fourth = hit().await.unwrap();

    assert_eq!(third, Answer::Allowed);
    assert!(matches!(fourth, Answer::Refused { .. }));
    // All three were counted in the window that was open, under the hash of version 1.
    assert_eq!(counters_by_key_version(&pool).await, [(Some(1), 3)]);
}

#[sqlx::test]
async fn a_new_window_goes_under_the_current_version_of_the_secret_key(pool: PgPool) {
    let (limits, hash) = limits_with_hash(&pool);
    hash.set_versions(&[2, 1]);

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

    assert_eq!(counters_by_key_version(&pool).await, [(Some(2), 1)]);
}

#[sqlx::test]
async fn with_the_old_version_gone_the_count_starts_again(pool: PgPool) {
    let (limits, hash) = limits_with_hash(&pool);
    let tenant = Uuid::new_v4();
    let hit = async || {
        limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .hidden("anna@firma.pl")
            .call()
            .await
            .unwrap()
            .hit()
            .await
    };
    for _ in 0..3 {
        let _counted = hit().await.unwrap();
    }

    // Version 1 was deleted from the source of keys while its window is still open.
    hash.set_versions(&[2]);
    let answer = hit().await.unwrap();

    assert_eq!(answer, Answer::Allowed);
    assert_eq!(
        counters_by_key_version(&pool).await,
        [(Some(1), 3), (Some(2), 1)]
    );
}

/// A keyed hash that breaks the contract of the trait: it returns what the test gives it.
struct BrokenHash(Vec<Vec<u8>>);

#[async_trait]
impl KeyedHash for BrokenHash {
    async fn hashes(&self, _data: &[u8]) -> Result<Vec<Vec<u8>>, KeyedHashError> {
        Ok(self.0.clone())
    }
}

#[sqlx::test]
async fn a_hash_longer_than_the_longest_allowed_is_an_error_and_counts_nothing(pool: PgPool) {
    // 128 bytes is the longest hash the module takes.
    let longest = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(BrokenHash(vec![vec![7; MAX_HASH_BYTES]]))
        .audit(common::NoAudit)
        .limits(common::all_limits())
        .run_jobs_in_background(|_error| {})
        .build();
    let answer = longest
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(Uuid::new_v4())
        .hidden("anna@firma.pl")
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap();
    assert_eq!(answer, Answer::Allowed);

    // One byte more, e.g. from a "hash" that returns its data unchanged, also as the hash of
    // an older version of the key.
    let too_long = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(BrokenHash(vec![vec![7; 8], vec![7; MAX_HASH_BYTES + 1]]))
        .audit(common::NoAudit)
        .limits(common::all_limits())
        .run_jobs_in_background(|_error| {})
        .build();
    let error = too_long
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(Uuid::new_v4())
        .hidden("anna@firma.pl")
        .call()
        .await
        .unwrap_err();

    assert!(matches!(error, LimitsError::KeyedHashGaveLongHash));
    assert_eq!(counters(&pool).await.len(), 1);
}

/// A keyed hash that cannot get its key.
struct FailingHash;

#[async_trait]
impl KeyedHash for FailingHash {
    async fn hashes(&self, _data: &[u8]) -> Result<Vec<Vec<u8>>, KeyedHashError> {
        Err(KeyedHashError::new("the key could not be fetched"))
    }
}

#[sqlx::test]
async fn a_keyed_hash_that_fails_is_an_error_and_counts_nothing(pool: PgPool) {
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(FailingHash)
        .audit(common::NoAudit)
        .limits(common::all_limits())
        .run_jobs_in_background(|_error| {})
        .build();

    let error = limits
        .attempts(TestLimit::ThreePerHour)
        .tenant_id(Uuid::new_v4())
        .hidden("anna@firma.pl")
        .call()
        .await
        .unwrap_err();

    assert!(matches!(error, LimitsError::KeyedHash(_)));
    assert_eq!(
        error.to_string(),
        "the keyed hash failed: the key could not be fetched"
    );
    assert_eq!(counters(&pool).await, []);
}

#[sqlx::test]
async fn a_keyed_hash_that_gives_no_hash_is_an_error_and_counts_nothing(pool: PgPool) {
    // No hash at all, and an empty hash: the second is how the table marks "no hidden part".
    for broken in [Vec::new(), vec![Vec::new()]] {
        let limits = Limits::builder()
            .pool(pool.clone())
            .keyed_hash(BrokenHash(broken))
            .audit(common::NoAudit)
            .limits(common::all_limits())
            .run_jobs_in_background(|_error| {})
            .build();
        let tenant = Uuid::new_v4();

        let error = limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .hidden("anna@firma.pl")
            .call()
            .await
            .unwrap_err();
        // A key with no hidden part does not ask the hash at all.
        let without_hidden_part = limits
            .attempts(TestLimit::ThreePerHour)
            .tenant_id(tenant)
            .call()
            .await
            .unwrap()
            .hit()
            .await
            .unwrap();

        assert!(matches!(error, LimitsError::KeyedHashGaveNoHash));
        assert_eq!(without_hidden_part, Answer::Allowed);
    }
    assert_eq!(counters(&pool).await.len(), 2);
}

#[sqlx::test]
async fn attempts_at_once_pass_no_more_than_the_window_lets_through(
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

    // Twenty rounds, each on a new key: one round alone could pass by luck.
    for _ in 0..20 {
        let tenant = Uuid::new_v4();
        let mut attempts = JoinSet::new();
        for _ in 0..40 {
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
        let answers = attempts.join_all().await;

        let allowed = answers
            .iter()
            .filter(|answer| **answer == Answer::Allowed)
            .count();
        assert_eq!(allowed, 3);
    }
    // Every counter stopped at three.
    let counts: Vec<i64> = counters(&pool)
        .await
        .into_iter()
        .map(|(_, count)| count)
        .collect();
    assert_eq!(counts, [3; 20]);
}

#[sqlx::test]
async fn a_limit_with_a_bucket_is_an_error_and_counts_nothing(pool: PgPool) {
    let limits = limits(&pool);

    let error = limits
        .attempts(TestLimit::Bucket)
        .tenant_id(Uuid::new_v4())
        .call()
        .await
        .unwrap()
        .hit()
        .await
        .unwrap_err();

    assert!(matches!(error, LimitsError::BucketNotImplemented));
    assert_eq!(counters(&pool).await, []);
}
