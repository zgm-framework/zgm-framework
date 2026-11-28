//! The settings of the module.

use std::time::Duration;

use crate::tenant_settings::{DurationDefault, TenantSettings, TenantSettingsError};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// How long a user waits for deletion unless the tenant set otherwise: 30 days, as in
/// Microsoft Entra ID; a tenant may set from a day to a year.
const USER_DELETION_WINDOW: DurationDefault = DurationDefault {
    value: Duration::from_secs(30 * DAY.as_secs()),
    min: Some(DAY),
    max: Some(Duration::from_secs(365 * DAY.as_secs())),
};

/// How long a tenant waits for deletion, with all its users, unless the tenant set otherwise:
/// 90 days, as AWS keeps a closed account; a tenant may set from a week to a year.
const TENANT_DELETION_WINDOW: DurationDefault = DurationDefault {
    value: Duration::from_secs(90 * DAY.as_secs()),
    min: Some(Duration::from_secs(7 * DAY.as_secs())),
    max: Some(Duration::from_secs(365 * DAY.as_secs())),
};

/// A setting of the module, which a tenant can change for itself. `as_ref` gives its name,
/// with the prefix of the module, e.g. `iam.user_deletion_window`; the application changes a
/// setting of a tenant under that name, so a typo does not compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::AsRefStr)]
#[strum(serialize_all = "snake_case", prefix = "iam.")]
pub enum IamSetting {
    /// How long a user waits for deletion, during which the deletion can be cancelled; 30
    /// days unless the tenant set otherwise, from a day to a year.
    UserDeletionWindow,
    /// How long a tenant waits for deletion, with all its users, during which the deletion
    /// can be cancelled; 90 days unless the tenant set otherwise, from a week to a year.
    TenantDeletionWindow,
}

impl IamSetting {
    /// Gives the settings of tenants the defaults built into the module, with their bounds.
    pub(crate) async fn give_defaults(
        settings: &dyn TenantSettings,
    ) -> Result<(), TenantSettingsError> {
        settings
            .add_default_duration(
                IamSetting::UserDeletionWindow.as_ref(),
                USER_DELETION_WINDOW,
            )
            .await?;
        settings
            .add_default_duration(
                IamSetting::TenantDeletionWindow.as_ref(),
                TENANT_DELETION_WINDOW,
            )
            .await
    }
}
