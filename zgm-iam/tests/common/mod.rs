//! What the tests share: ciphers that behave in known ways, settings that remember what the
//! module gave them, and small helpers.
#![allow(
    dead_code,
    reason = "every file of tests is a program of its own and uses only some of these"
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;
use zgm_iam::{
    Cipher, CipherError, DurationDefault, Iam, State, Tenant, TenantSettings, TenantSettingsError,
    User,
};

/// What [`FlippingCipher`] makes of `data`: every bit flipped, so the bytes differ from the
/// text they came from and are easy to compute in a test.
pub fn flipped(data: &[u8]) -> Vec<u8> {
    data.iter().map(|byte| !byte).collect()
}

/// A cipher whose ciphertext is [`flipped`]. It counts its calls in the counter it is given.
#[derive(Default)]
pub struct FlippingCipher {
    pub calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Cipher for FlippingCipher {
    async fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, CipherError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(flipped(data))
    }

    async fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CipherError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(flipped(data))
    }
}

/// A cipher whose key store does not answer.
pub struct FailingCipher;

#[async_trait]
impl Cipher for FailingCipher {
    async fn encrypt(&self, _data: &[u8]) -> Result<Vec<u8>, CipherError> {
        Err(CipherError::new("the key store did not answer"))
    }

    async fn decrypt(&self, _data: &[u8]) -> Result<Vec<u8>, CipherError> {
        Err(CipherError::new("the key store did not answer"))
    }
}

/// A broken cipher: it gives empty bytes for everything.
pub struct EmptyCipher;

#[async_trait]
impl Cipher for EmptyCipher {
    async fn encrypt(&self, _data: &[u8]) -> Result<Vec<u8>, CipherError> {
        Ok(Vec::new())
    }

    async fn decrypt(&self, _data: &[u8]) -> Result<Vec<u8>, CipherError> {
        Ok(Vec::new())
    }
}

/// Settings of tenants that remember, in order, every default the module gives them and
/// every question it asks, and give a tenant its own value when the test set one, and the
/// last default of the setting otherwise: what an application without a module of settings
/// could give.
#[derive(Clone, Default)]
pub struct RecordedSettings {
    pub defaults: Arc<Mutex<Vec<(String, DurationDefault)>>>,
    pub of_tenants: Arc<Mutex<HashMap<(Uuid, String), Duration>>>,
    pub asked: Arc<Mutex<Vec<(Uuid, String)>>>,
}

impl RecordedSettings {
    /// Gives the tenant a value of its own, as a row of `zgm-settings` would.
    pub fn set(&self, tenant_id: Uuid, name: &str, value: Duration) {
        self.of_tenants
            .lock()
            .unwrap()
            .insert((tenant_id, name.to_owned()), value);
    }
}

#[async_trait]
impl TenantSettings for RecordedSettings {
    async fn add_default_duration(
        &self,
        name: &str,
        default: DurationDefault,
    ) -> Result<(), TenantSettingsError> {
        self.defaults
            .lock()
            .unwrap()
            .push((name.to_owned(), default));
        Ok(())
    }

    async fn duration(&self, tenant_id: Uuid, name: &str) -> Result<Duration, TenantSettingsError> {
        self.asked
            .lock()
            .unwrap()
            .push((tenant_id, name.to_owned()));
        if let Some(value) = self
            .of_tenants
            .lock()
            .unwrap()
            .get(&(tenant_id, name.to_owned()))
        {
            return Ok(*value);
        }
        self.defaults
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(given, _)| given == name)
            .map(|(_, default)| default.value)
            .ok_or_else(|| TenantSettingsError::new(format!("no default was given for {name}")))
    }
}

/// Settings whose database stops answering once the module is built: the defaults are
/// written, and every value asked for is an error.
pub struct FailingSettings;

#[async_trait]
impl TenantSettings for FailingSettings {
    async fn add_default_duration(
        &self,
        _name: &str,
        _default: DurationDefault,
    ) -> Result<(), TenantSettingsError> {
        Ok(())
    }

    async fn duration(
        &self,
        _tenant_id: Uuid,
        _name: &str,
    ) -> Result<Duration, TenantSettingsError> {
        Err(TenantSettingsError::new("the settings did not answer"))
    }
}

/// Settings that cannot write a default, e.g. their database does not answer when the
/// application starts.
pub struct UnwritableSettings;

#[async_trait]
impl TenantSettings for UnwritableSettings {
    async fn add_default_duration(
        &self,
        _name: &str,
        _default: DurationDefault,
    ) -> Result<(), TenantSettingsError> {
        Err(TenantSettingsError::new("the settings could not write"))
    }

    async fn duration(
        &self,
        _tenant_id: Uuid,
        _name: &str,
    ) -> Result<Duration, TenantSettingsError> {
        Err(TenantSettingsError::new("the settings could not write"))
    }
}

/// The module on a migrated database, with the given settings and a [`FlippingCipher`].
pub async fn iam_with_settings(pool: &PgPool, settings: impl TenantSettings) -> Iam {
    zgm_iam::migrate(pool).await.unwrap();
    Iam::builder()
        .pool(pool.clone())
        .cipher(FlippingCipher::default())
        .settings(settings)
        .run_jobs_in_background(|_error| {})
        .build()
        .await
        .unwrap()
}

/// The module on a migrated database, with the given cipher.
pub async fn iam_with(pool: &PgPool, cipher: impl Cipher) -> Iam {
    zgm_iam::migrate(pool).await.unwrap();
    Iam::builder()
        .pool(pool.clone())
        .cipher(cipher)
        .settings(RecordedSettings::default())
        .run_jobs_in_background(|_error| {})
        .build()
        .await
        .unwrap()
}

/// How many rows the table of tenants has.
pub async fn count_tenants(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM zgm_iam.tenants")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// How many rows the table of users has.
pub async fn count_users(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM zgm_iam.users")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Puts the word of a state into the row of a tenant, by hand: no function of the module
/// changes the state of a tenant yet.
pub async fn set_state_of_tenant(pool: &PgPool, tenant_id: uuid::Uuid, word: &str) {
    sqlx::query("UPDATE zgm_iam.tenants SET state = $2 WHERE id = $1")
        .bind(tenant_id)
        .bind(word)
        .execute(pool)
        .await
        .unwrap();
}

/// Puts the word of a state into the row of a user, by hand, also one the module does not
/// know.
pub async fn set_state_of_user(pool: &PgPool, user_id: uuid::Uuid, word: &str) {
    sqlx::query("UPDATE zgm_iam.users SET state = $2 WHERE id = $1")
        .bind(user_id)
        .bind(word)
        .execute(pool)
        .await
        .unwrap();
}

/// A UUID v7 that is the id of nothing in the module's tables.
pub async fn an_id_of_nothing(pool: &PgPool) -> uuid::Uuid {
    sqlx::query_scalar("SELECT uuidv7()")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The user as its row in the table says, read by hand and with the words of the states
/// mapped by hand, not by the module.
pub async fn row_of(pool: &PgPool, user_id: Uuid) -> User {
    let (id, tenant_id, state, deletes_at, created_at, updated_at): (
        Uuid,
        Uuid,
        String,
        Option<jiff_sqlx::Timestamp>,
        jiff_sqlx::Timestamp,
        jiff_sqlx::Timestamp,
    ) = sqlx::query_as(
        "SELECT id, tenant_id, state, deletes_at, created_at, updated_at \
         FROM zgm_iam.users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let state = match state.as_str() {
        "active" => State::Active,
        "suspended" => State::Suspended,
        "pending_deletion" => State::PendingDeletion,
        other => panic!("no state is called {other}"),
    };
    User {
        id,
        tenant_id,
        state,
        deletes_at: deletes_at.map(|date| date.to_jiff()),
        created_at: created_at.to_jiff(),
        updated_at: updated_at.to_jiff(),
    }
}

/// The tenant as its row in the table says, read by hand, with the name decrypted by
/// [`flipped`] and the words of the states mapped by hand, not by the module.
pub async fn tenant_row_of(pool: &PgPool, tenant_id: Uuid) -> Tenant {
    let (id, name, state, deletes_at, created_at, updated_at): (
        Uuid,
        Vec<u8>,
        String,
        Option<jiff_sqlx::Timestamp>,
        jiff_sqlx::Timestamp,
        jiff_sqlx::Timestamp,
    ) = sqlx::query_as(
        "SELECT id, name, state, deletes_at, created_at, updated_at \
         FROM zgm_iam.tenants WHERE id = $1",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .unwrap();
    let state = match state.as_str() {
        "active" => State::Active,
        "suspended" => State::Suspended,
        "pending_deletion" => State::PendingDeletion,
        other => panic!("no state is called {other}"),
    };
    Tenant {
        id,
        name: String::from_utf8(flipped(&name)).unwrap(),
        state,
        deletes_at: deletes_at.map(|date| date.to_jiff()),
        created_at: created_at.to_jiff(),
        updated_at: updated_at.to_jiff(),
    }
}

/// A cipher whose decryption gives bytes that are not UTF-8, e.g. the key of another
/// application.
pub struct GarbledCipher;

#[async_trait]
impl Cipher for GarbledCipher {
    async fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, CipherError> {
        Ok(flipped(data))
    }

    async fn decrypt(&self, _data: &[u8]) -> Result<Vec<u8>, CipherError> {
        Ok(vec![0xff, 0xfe])
    }
}
