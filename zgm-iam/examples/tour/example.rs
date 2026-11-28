//! What an application writes to use the module: the glue to `zgm-settings`, and a cipher.

use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;
use zgm_iam::{Cipher, CipherError, DurationDefault, TenantSettings, TenantSettingsError};
use zgm_settings::{DefaultSetting, Value};

/// The glue from the module to `zgm-settings`: the defaults of the module go to the
/// database of the settings, and the value a tenant has comes back from there.
pub struct IamSettings(pub zgm_settings::Settings);

#[async_trait]
impl TenantSettings for IamSettings {
    async fn add_default_duration(
        &self,
        name: &str,
        default: DurationDefault,
    ) -> Result<(), TenantSettingsError> {
        let DurationDefault { value, min, max } = default;
        let default = DefaultSetting::Duration { value, min, max };
        self.0
            .add_default(name, default)
            .await
            .map(|_| ())
            .map_err(TenantSettingsError::new)
    }

    async fn duration(&self, tenant_id: Uuid, name: &str) -> Result<Duration, TenantSettingsError> {
        match self
            .0
            .tenant_id(tenant_id)
            .get(name)
            .await
            .map_err(TenantSettingsError::new)?
        {
            Value::Duration(duration) => Ok(duration),
            other => Err(TenantSettingsError::new(format!(
                "{name} is not a duration: {other:?}"
            ))),
        }
    }
}

/// A cipher for the tour only: it flips every bit, which hides nothing. An application gives
/// its real cipher here, e.g. one that asks its key service, so that the names of the tenants
/// are not readable in the database.
pub struct ToyCipher;

#[async_trait]
impl Cipher for ToyCipher {
    async fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, CipherError> {
        Ok(data.iter().map(|byte| !byte).collect())
    }

    async fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CipherError> {
        Ok(data.iter().map(|byte| !byte).collect())
    }
}

pub const DAY: Duration = Duration::from_secs(24 * 60 * 60);
