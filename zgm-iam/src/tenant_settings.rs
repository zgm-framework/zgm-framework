//! The settings of tenants the application gives the module.

use std::error::Error;
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

/// The default of a setting that is a length of time, with the shortest and the longest a
/// tenant may set; `None` is no bound. The fields are named, so that the two bounds cannot be
/// swapped by their order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurationDefault {
    /// The value of every tenant that has not changed the setting.
    pub value: Duration,
    /// The shortest a tenant may set, or `None` for no bound.
    pub min: Option<Duration>,
    /// The longest a tenant may set, or `None` for no bound.
    pub max: Option<Duration>,
}

/// Where the module keeps the defaults of its settings and reads the value a tenant has, e.g.
/// how long a user of the tenant waits for deletion. The application gives an implementation
/// to the builder of [`Iam`](crate::Iam), usually a few lines that hand the calls on to
/// `zgm-settings`; the module depends on no other module.
///
/// ```ignore
/// use zgm_settings::{DefaultSetting, Value};
///
/// struct IamSettings(zgm_settings::Settings);
///
/// #[async_trait]
/// impl TenantSettings for IamSettings {
///     async fn add_default_duration(&self, name: &str, default: DurationDefault) -> Result<(), TenantSettingsError> {
///         let DurationDefault { value, min, max } = default;
///         let default = DefaultSetting::Duration { value, min, max };
///         self.0.add_default(name, default).await.map(|_| ()).map_err(TenantSettingsError::new)
///     }
///
///     async fn duration(&self, tenant_id: Uuid, name: &str) -> Result<Duration, TenantSettingsError> {
///         match self.0.tenant_id(tenant_id).get(name).await.map_err(TenantSettingsError::new)? {
///             Value::Duration(duration) => Ok(duration),
///             other => Err(TenantSettingsError::new(format!("{name} is not a duration: {other:?}"))),
///         }
///     }
/// }
/// ```
///
/// The names are those of [`IamSetting`](crate::IamSetting), e.g. `iam.user_deletion_window`.
/// So far every setting of the module is a length of time.
#[async_trait]
pub trait TenantSettings: Send + Sync + 'static {
    /// Gives the default of a setting that is a length of time, with its bounds: the value of
    /// every tenant that has not changed it. The module calls it when it is built, once for
    /// each of its settings, with the numbers built into the module. `zgm-settings` writes
    /// them to its database the first time and leaves them there after that.
    ///
    /// An error, e.g. the database did not answer, is the error of
    /// [`build`](crate::Iam::builder), and the module is not built.
    async fn add_default_duration(
        &self,
        name: &str,
        default: DurationDefault,
    ) -> Result<(), TenantSettingsError>;

    /// The length of time the tenant has for a setting: its own, or the default.
    ///
    /// An error, e.g. the database did not answer, is the error of the operation that asked.
    async fn duration(&self, tenant_id: Uuid, name: &str) -> Result<Duration, TenantSettingsError>;
}

/// Why a [`TenantSettings`] could not give a value. It wraps the implementation's own error.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct TenantSettingsError(Box<dyn Error + Send + Sync>);

impl TenantSettingsError {
    /// Wraps the error of an implementation.
    pub fn new(error: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self(error.into())
    }
}
