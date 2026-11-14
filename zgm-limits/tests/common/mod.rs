//! What the tests share: their limits, a clock they move, and small helpers.
#![allow(
    dead_code,
    reason = "every file of tests is a program of its own and uses only some of these"
)]

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use sqlx::{PgConnection, PgPool};
use strum::VariantArray;
use zgm_limits::{
    Answer, Audit, AuditError, Bounds, Clock, Count, KeyedHash, KeyedHashError, Limit, Limits,
    LimitsEvent, Period, Rule,
};

/// The limits of the tests.
#[derive(Debug, Clone, Copy, strum::IntoStaticStr, strum::VariantArray, strum::EnumProperty)]
#[strum(serialize_all = "snake_case")]
pub enum TestLimit {
    /// A window nobody waits out in a test. A tenant may set 1 to 10 attempts per one minute
    /// to one day.
    #[strum(props(
        min_at_most = 1,
        max_at_most = 10,
        min_per_seconds = 60,
        max_per_seconds = 86400
    ))]
    ThreePerHour,
    /// The same rule under another name, and with no bounds: a tenant cannot change it.
    OtherThreePerHour,
    /// A window full after one attempt, with the same bounds as `ThreePerHour`.
    #[strum(props(
        min_at_most = 1,
        max_at_most = 10,
        min_per_seconds = 60,
        max_per_seconds = 86400
    ))]
    OnePerHour,
    /// A way of counting that is not implemented yet.
    Bucket,
    /// A window of distinct elements, with the same bounds as `ThreePerHour`.
    #[strum(props(
        min_at_most = 1,
        max_at_most = 10,
        min_per_seconds = 60,
        max_per_seconds = 86400
    ))]
    ThreeDistinctPerHour,
}

/// The bounds of `ThreePerHour` and `OnePerHour`.
pub fn test_bounds() -> Bounds {
    Bounds {
        min_at_most: Count::new(1),
        max_at_most: Count::new(10),
        min_per: Period::minutes(1),
        max_per: Period::days(1),
    }
}

/// An audit that takes every event and keeps none, for the tests that are not about events.
pub struct NoAudit;

#[async_trait]
impl Audit for NoAudit {
    async fn record_in_transaction(
        &self,
        _transaction: &mut PgConnection,
        _event: LimitsEvent,
    ) -> Result<(), AuditError> {
        Ok(())
    }

    fn record_after_the_fact(&self, _event: LimitsEvent) {}
}

/// An audit that keeps every event, for the tests that are about them.
#[derive(Clone, Default)]
pub struct RecordedAudit(Arc<Mutex<Vec<LimitsEvent>>>);

impl RecordedAudit {
    /// The events so far, taken out: the next call gives only what came after.
    pub fn take(&self) -> Vec<LimitsEvent> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

#[async_trait]
impl Audit for RecordedAudit {
    async fn record_in_transaction(
        &self,
        _transaction: &mut PgConnection,
        event: LimitsEvent,
    ) -> Result<(), AuditError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }

    fn record_after_the_fact(&self, event: LimitsEvent) {
        self.0.lock().unwrap().push(event);
    }
}

/// All limits of the tests, for the builder.
pub fn all_limits() -> &'static [TestLimit] {
    TestLimit::VARIANTS
}

impl Limit for TestLimit {
    fn rule(self) -> Rule {
        match self {
            Self::ThreePerHour | Self::OtherThreePerHour => Rule::Window {
                at_most: Count::new(3),
                per: Period::hours(1),
            },
            Self::OnePerHour => Rule::Window {
                at_most: Count::new(1),
                per: Period::hours(1),
            },
            Self::Bucket => Rule::Bucket {
                capacity: Count::new(20),
                refill_every: Period::seconds(3),
            },
            Self::ThreeDistinctPerHour => Rule::WindowOfDistinct {
                at_most: Count::new(3),
                per: Period::hours(1),
            },
        }
    }
}

/// A hash for tests, with versions of its key that a test changes. A hash is laid out as
/// `zgm-encryption` writes it: the format byte, the version of the key in four bytes, and then
/// the hash itself. It is not a real keyed hash; it only gives another value for other data
/// and for another version.
#[derive(Debug, Clone)]
pub struct TestHash(Arc<Mutex<Vec<u32>>>);

impl TestHash {
    /// A hash whose key has one version, 1.
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(vec![1])))
    }

    /// The versions of the key that can be read from now on, the current one first.
    pub fn set_versions(&self, versions: &[u32]) {
        *self.0.lock().unwrap() = versions.to_vec();
    }
}

#[async_trait]
impl KeyedHash for TestHash {
    async fn hashes(&self, data: &[u8]) -> Result<Vec<Vec<u8>>, KeyedHashError> {
        let versions = self.0.lock().unwrap().clone();
        Ok(versions
            .into_iter()
            .map(|version| {
                let mut hasher = DefaultHasher::new();
                (version, data).hash(&mut hasher);
                let mut bytes = vec![1];
                bytes.extend_from_slice(&version.to_be_bytes());
                bytes.extend_from_slice(&hasher.finish().to_be_bytes());
                bytes
            })
            .collect())
    }
}

/// The version of the key a hash from [`TestHash`] was computed with; `None` for the empty
/// hash of a key with no hidden part.
pub fn key_version(hash: &[u8]) -> Option<u32> {
    let version = hash.get(1..5)?;
    Some(u32::from_be_bytes(version.try_into().unwrap()))
}

/// A clock that stands still until a test moves it.
#[derive(Debug, Clone)]
pub struct TestClock(Arc<Mutex<DateTime<Utc>>>);

impl TestClock {
    /// A clock that shows noon.
    pub fn at_noon() -> Self {
        Self(Arc::new(Mutex::new(noon())))
    }

    pub fn advance(&self, by: Duration) {
        *self.0.lock().unwrap() += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }
}

/// The moment the test clock starts at.
pub fn noon() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()
}

pub const HOUR: Duration = Duration::from_hours(1);
pub const MINUTE: Duration = Duration::from_mins(1);

/// The module with a clock the test moves.
pub fn limits_at_noon(pool: &PgPool) -> (Limits, TestClock) {
    let clock = TestClock::at_noon();
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(TestHash::new())
        .clock(clock.clone())
        .audit(NoAudit)
        .limits(all_limits())
        .run_jobs_in_background(|_error| {})
        .build();
    (limits, clock)
}

pub fn limits(pool: &PgPool) -> Limits {
    limits_with_hash(pool).0
}

/// The module with a hash whose key versions the test changes.
pub fn limits_with_hash(pool: &PgPool) -> (Limits, TestHash) {
    let hash = TestHash::new();
    let limits = Limits::builder()
        .pool(pool.clone())
        .keyed_hash(hash.clone())
        .audit(NoAudit)
        .limits(all_limits())
        .run_jobs_in_background(|_error| {})
        .build();
    (limits, hash)
}

/// The end of the window of a refused attempt.
pub fn refused_until(answer: Answer) -> DateTime<Utc> {
    match answer {
        Answer::Refused { until } => until,
        Answer::Allowed => panic!("the attempt passed, a refusal was expected"),
    }
}

/// The hash and the count of every counter, in the order of the hashes.
pub async fn counters(pool: &PgPool) -> Vec<(Vec<u8>, i64)> {
    sqlx::query_as(
        "SELECT key_hash, (state->>'count')::bigint \
         FROM zgm_limits.counters ORDER BY key_hash",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}
