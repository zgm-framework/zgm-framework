//! The entry point of the module.

use std::fmt;
use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use crate::cipher::Cipher;
use crate::error::IamError;
use crate::iam_setting::IamSetting;
use crate::jobs::{JobError, OnJobError, StopJobsOnDrop};
use crate::repository;
use crate::tenant::Tenant;
use crate::tenant_scope::TenantScope;
use crate::tenant_settings::TenantSettings;

/// The module: everything the application does with tenants and their users goes through it.
///
/// ```ignore
/// let iam = Iam::builder()
///     .pool(pool)
///     .cipher(cipher)
///     .settings(IamSettings(settings))
///     .run_jobs_in_background(|error: JobError| eprintln!("{}: {}", error.job, error.error))
///     .build()
///     .await?;
/// let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await?;
/// let anna = iam.tenant_id(kowalski.id).create_user().await?;
/// ```
///
/// The functions of one tenant hang from [`Iam::tenant_id`]; here are only those that work
/// above the tenants, such as [`Iam::create_tenant`], whose tenant has no id yet.
///
/// The application builds it once, when it starts, after [`migrate`](crate::migrate), and
/// clones it where it needs it; a clone shares the pool, the cipher, the settings and the loop
/// that deletes the users past their date, which stops with the last clone.
#[derive(Clone)]
pub struct Iam {
    pool: PgPool,
    cipher: Arc<dyn Cipher>,
    settings: Arc<dyn TenantSettings>,
    /// `None` only in the loop's own copy, so that the loop does not keep itself running.
    stop_jobs: Option<Arc<StopJobsOnDrop>>,
}

/// Without the cipher, which holds secret keys, and the settings.
impl fmt::Debug for Iam {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Iam")
            .field("pool", &self.pool)
            .finish_non_exhaustive()
    }
}

#[bon::bon]
impl Iam {
    /// Builds the module on the application's pool of connections, and gives the settings
    /// the defaults of the module's settings ([`IamSetting`]), with their bounds, which
    /// `zgm-settings` writes to its database the first time.
    ///
    /// [`IamError::Settings`] when the settings cannot take a default, e.g. their database did
    /// not answer; the module is not built then, and no loop starts. Called outside a `tokio`
    /// runtime, it panics.
    #[builder]
    pub async fn new(
        pool: PgPool,
        /// Encrypts what the module keeps readable nowhere, such as the names of tenants.
        /// Always required.
        #[builder(with = |cipher: impl Cipher| Arc::new(cipher) as Arc<dyn Cipher>)]
        cipher: Arc<dyn Cipher>,
        /// Keeps the defaults of the module's settings and gives the value a tenant has.
        /// Always required.
        #[builder(with = |settings: impl TenantSettings| Arc::new(settings) as Arc<dyn TenantSettings>)]
        settings: Arc<dyn TenantSettings>,
        /// Starts the loop that deletes the users past their date, every two minutes, and
        /// says what to do when a pass fails, e.g. write it to the log; the loop goes on.
        /// Always required, so that the loop cannot be forgotten.
        #[builder(with = |on_error: impl Fn(JobError) + Send + Sync + 'static| Box::new(on_error) as OnJobError)]
        run_jobs_in_background: OnJobError,
    ) -> Result<Self, IamError> {
        IamSetting::give_defaults(settings.as_ref()).await?;
        let mut iam = Self {
            pool,
            cipher,
            settings,
            stop_jobs: None,
        };
        let task = tokio::spawn(iam.clone().run_jobs(run_jobs_in_background));
        iam.stop_jobs = Some(Arc::new(StopJobsOnDrop(task.abort_handle())));
        Ok(iam)
    }

    /// The module within one tenant, where all the functions of a tenant are. It does not
    /// ask the database whether the tenant exists: the first function called says so.
    pub fn tenant_id(&self, tenant_id: Uuid) -> TenantScope {
        TenantScope::new(self.clone(), tenant_id)
    }

    /// The pool of connections, for the functions of one tenant.
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The tenant a row is, with its name decrypted by the application's cipher.
    /// [`IamError::UnreadableName`] when the cipher gives bytes that are not UTF-8.
    pub(crate) async fn tenant_of(&self, row: repository::TenantRow) -> Result<Tenant, IamError> {
        let name = self.cipher.decrypt(&row.name).await?;
        let name = String::from_utf8(name).map_err(|_| IamError::UnreadableName)?;
        Ok(Tenant {
            id: row.id,
            name,
            state: row.state,
            deletes_at: row.deletes_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// The settings of tenants, for the functions of one tenant.
    pub(crate) fn settings(&self) -> &dyn TenantSettings {
        self.settings.as_ref()
    }

    /// Creates a tenant with the given name and gives it back, with the id, the state and the
    /// dates the database gave it. A new tenant is [`State::Active`](crate::State::Active)
    /// and waits for no deletion.
    ///
    /// The table gets only what the [`Cipher`] made of the name. The module checks one thing
    /// about the name: that it is not empty. Its length, its characters and spaces at its
    /// ends are the application's business, and two tenants may have the same name.
    ///
    /// Nothing is written when the name is empty ([`IamError::EmptyName`], and the cipher is
    /// not called), when the cipher fails ([`IamError::Cipher`]), or when it gives empty
    /// bytes ([`IamError::EmptyCiphertext`]). When the table gives a new row a state the
    /// module does not know ([`IamError::UnknownState`]), which only SQL written by hand can
    /// cause, the tenant is already written.
    pub async fn create_tenant(&self, name: &str) -> Result<Tenant, IamError> {
        if name.is_empty() {
            return Err(IamError::EmptyName);
        }
        let ciphertext = self.cipher.encrypt(name.as_bytes()).await?;
        if ciphertext.is_empty() {
            return Err(IamError::EmptyCiphertext);
        }
        let mut connection = self.pool.acquire().await?;
        let row = repository::insert_tenant(&mut connection, &ciphertext).await?;
        Ok(Tenant {
            id: row.id,
            name: name.to_owned(),
            state: row.state,
            deletes_at: row.deletes_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}
