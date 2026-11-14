//! The product side of the tour: what an application writes to use the module.
//!
//! 1. its limits, in an enum, each with a rule ([`AppLimit`]);
//! 2. a keyed hash for the hidden parts of keys ([`ExampleHash`]);
//! 3. a place for the module's events ([`AppAudit`]);
//! 4. the calls around the action a limit guards ([`sign_in`]).

use std::hash::{DefaultHasher, Hash, Hasher};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;
use zgm_limits::{
    Answer, Audit, AuditError, Count, KeyedHash, KeyedHashError, Limit, Limits, LimitsError,
    LimitsEvent, Period, Rule,
};

// ---------------------------------------------------------------------------------------------
// 1. The limits of the product.
// ---------------------------------------------------------------------------------------------

/// All limits of the product. A new limit is a new variant here and its rule below; nothing is
/// registered anywhere else.
///
/// `strum` gives every limit its name (`wrong_password`), under which its counters lie in the
/// database, the list of all of them (`AppLimit::VARIANTS`), and the bounds within which a
/// tenant may change its rule (`props`: all four or none).
#[derive(Debug, Clone, Copy, strum::IntoStaticStr, strum::VariantArray, strum::EnumProperty)]
#[strum(serialize_all = "snake_case")]
pub enum AppLimit {
    /// Wrong passwords at sign-in, counted per account. A tenant may set 1 to 20 attempts per
    /// one minute to one day.
    #[strum(props(
        min_at_most = 1,
        max_at_most = 20,
        min_per_seconds = 60,
        max_per_seconds = 86400
    ))]
    WrongPassword,
    /// One-time codes sent to an e-mail address, counted per address. The address is the
    /// hidden part of the key: the database holds only its hash.
    SignInCode,
    /// Data exports, counted for the whole tenant. No bounds: a tenant cannot change it.
    Export,
    /// Different accounts somebody tries to sign in to from one address: one password tried
    /// on many accounts gives each account one attempt, which no limit per account notices.
    AccountsPerAddress,
    /// Searches. A bucket with tokens, which the module does not count yet.
    Search,
}

impl Limit for AppLimit {
    fn rule(self) -> Rule {
        match self {
            // The first wrong password opens a window of 15 minutes; three fit in it.
            Self::WrongPassword => Rule::Window {
                at_most: Count::new(3),
                per: Period::minutes(15),
            },
            Self::SignInCode => Rule::Window {
                at_most: Count::new(3),
                per: Period::minutes(10),
            },
            Self::Export => Rule::Window {
                at_most: Count::new(2),
                per: Period::hours(1),
            },
            // A window filled by different accounts, not by attempts: three in ten minutes.
            Self::AccountsPerAddress => Rule::WindowOfDistinct {
                at_most: Count::new(3),
                per: Period::minutes(10),
            },
            Self::Search => Rule::Bucket {
                capacity: Count::new(20),
                refill_every: Period::seconds(3),
            },
        }
    }
}

// ---------------------------------------------------------------------------------------------
// 2. The keyed hash.
// ---------------------------------------------------------------------------------------------

/// A stand-in for a keyed hash, so that the example needs no secret key: the hash of the
/// standard library, which has no key at all. **Not for a real application**: anybody with a
/// copy of the database could find the e-mails by trying them one by one. A real application
/// gives HMAC with a secret key here, e.g. HMAC-SHA-256.
pub struct ExampleHash;

#[async_trait]
impl KeyedHash for ExampleHash {
    async fn hashes(&self, data: &[u8]) -> Result<Vec<Vec<u8>>, KeyedHashError> {
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        // One hash: this stand-in has one "version of the key". An implementation with a key
        // that changes returns one hash for every version it can still read, the current
        // version first, and the module finds a counter written with any of them.
        Ok(vec![hasher.finish().to_be_bytes().to_vec()])
    }
}

// ---------------------------------------------------------------------------------------------
// 3. The audit.
// ---------------------------------------------------------------------------------------------

/// The application's audit. The module has no logging of its own: everything it would log, it
/// reports here, in one of two ways.
pub struct AppAudit;

impl AppAudit {
    /// Creates the application's own audit table. It is not a table of the module: the
    /// application decides what its audit looks like.
    pub async fn create_table(pool: &PgPool) -> Result<(), sqlx::Error> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS app_audit ( \
                 id        bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, \
                 at        timestamptz NOT NULL DEFAULT statement_timestamp(), \
                 tenant_id uuid NOT NULL, \
                 event     text NOT NULL)",
        )
        .execute(pool)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl Audit for AppAudit {
    /// A change somebody made: a block lifted, a rule changed. `transaction` is the open
    /// transaction of that change, so the row written with it is committed together with the
    /// change, or not at all. An error returned from here rolls the change back.
    async fn record_in_transaction(
        &self,
        transaction: &mut PgConnection,
        event: LimitsEvent,
    ) -> Result<(), AuditError> {
        let tenant_id = match &event {
            LimitsEvent::BlockLifted { key, .. } => key.tenant_id,
            LimitsEvent::RuleChanged { tenant_id, .. } => *tenant_id,
            // The other events never come this way.
            _ => Uuid::nil(),
        };
        println!("    [audit, in the transaction] {}", described(&event));
        sqlx::query("INSERT INTO app_audit (tenant_id, event) VALUES ($1, $2)")
            .bind(tenant_id)
            .bind(described(&event))
            .execute(transaction)
            .await
            .map_err(AuditError::new)?;
        Ok(())
    }

    /// A refusal, a key that an attempt blocked, a pass of the cleanup: like a line of a log.
    /// It must return at once, so a real application sends the event to a queue here and
    /// writes it somewhere else; this one prints it.
    fn record_after_the_fact(&self, event: LimitsEvent) {
        println!("    [audit, after the fact] {}", described(&event));
    }
}

/// An event in a few words. No event holds the hidden part of a key or its hash, only whether
/// there was one.
fn described(event: &LimitsEvent) -> String {
    match event {
        LimitsEvent::AttemptRefused { key, until } => format!(
            "AttemptRefused in `{}` until {}",
            key.limit_name,
            until.format("%H:%M:%S")
        ),
        LimitsEvent::KeyBlocked { key, until } => format!(
            "KeyBlocked in `{}` until {}",
            key.limit_name,
            until.format("%H:%M:%S")
        ),
        LimitsEvent::BlockLifted { key, by } => {
            format!("BlockLifted in `{}` by {by:?}", key.limit_name)
        }
        LimitsEvent::RuleChanged {
            limit_name, rule, ..
        } => match rule {
            Some(rule) => format!("RuleChanged of `{limit_name}` to {rule:?}"),
            None => format!("RuleChanged of `{limit_name}` back to the default"),
        },
        LimitsEvent::ExpiredDeleted { counters } => {
            format!("ExpiredDeleted, {counters} counters")
        }
    }
}

// ---------------------------------------------------------------------------------------------
// 4. Using a limit.
// ---------------------------------------------------------------------------------------------

/// The password every account of the example has.
const RIGHT_PASSWORD: &str = "correct horse";

/// How a sign-in ended.
#[derive(Debug)]
pub enum SignIn {
    /// The password was right.
    SignedIn,
    /// The password was wrong; the attempt was counted before the check.
    WrongPassword,
    /// The account is blocked; the password was not even checked.
    TooManyAttempts {
        /// The moment from which the account may try again.
        until: DateTime<Utc>,
    },
}

/// A sign-in guarded by [`AppLimit::WrongPassword`]: count the attempt first, check the
/// password only when it passed, and forget the attempts after a success.
pub async fn sign_in(
    limits: &Limits,
    tenant_id: Uuid,
    account_id: Uuid,
    password: &str,
) -> Result<SignIn, LimitsError> {
    // Whose attempts in which limit: built once, used for both calls below.
    let attempts = limits
        .attempts(AppLimit::WrongPassword)
        .tenant_id(tenant_id)
        .subject_id(account_id)
        .call()
        .await?;

    // Before the password is checked: count the attempt. Counting and checking the limit are
    // one statement, so of many sign-ins sent at once only three get any further.
    if let Answer::Refused { until } = attempts.hit().await? {
        return Ok(SignIn::TooManyAttempts { until });
    }

    if password == RIGHT_PASSWORD {
        // After a success: forget the attempts, so the next mistake starts from nothing.
        attempts.reset().await?;
        Ok(SignIn::SignedIn)
    } else {
        // The failure is already counted. The third one in a window blocks the account.
        Ok(SignIn::WrongPassword)
    }
}
