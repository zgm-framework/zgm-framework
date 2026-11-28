//! The settings of tenants: what the module gives the application's `TenantSettings` when it
//! is built.

mod common;

use std::time::Duration;

use sqlx::PgPool;
use zgm_iam::{DurationDefault, Iam, IamError, IamSetting};

use common::{FlippingCipher, RecordedSettings, UnwritableSettings};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

#[sqlx::test(migrations = false)]
async fn building_the_module_gives_the_defaults_of_both_deletion_windows_with_their_bounds_once(
    pool: PgPool,
) {
    let settings = RecordedSettings::default();

    let _iam = Iam::builder()
        .pool(pool)
        .cipher(FlippingCipher::default())
        .settings(settings.clone())
        .run_jobs_in_background(|_error| {})
        .build()
        .await
        .unwrap();

    assert_eq!(
        *settings.defaults.lock().unwrap(),
        [
            (
                "iam.user_deletion_window".to_owned(),
                DurationDefault {
                    value: DAY * 30,
                    min: Some(DAY),
                    max: Some(DAY * 365),
                }
            ),
            (
                "iam.tenant_deletion_window".to_owned(),
                DurationDefault {
                    value: DAY * 90,
                    min: Some(DAY * 7),
                    max: Some(DAY * 365),
                }
            ),
        ],
    );
}

#[sqlx::test(migrations = false)]
async fn a_clone_of_the_module_gives_no_defaults_again(pool: PgPool) {
    let settings = RecordedSettings::default();
    let iam = Iam::builder()
        .pool(pool)
        .cipher(FlippingCipher::default())
        .settings(settings.clone())
        .run_jobs_in_background(|_error| {})
        .build()
        .await
        .unwrap();

    let _clone = iam.clone();
    let _in_a_tenant = iam.tenant_id(uuid::Uuid::nil());

    assert_eq!(settings.defaults.lock().unwrap().len(), 2);
}

#[sqlx::test(migrations = false)]
async fn when_the_settings_cannot_write_the_default_building_is_an_error(pool: PgPool) {
    let built = Iam::builder()
        .pool(pool)
        .cipher(FlippingCipher::default())
        .settings(UnwritableSettings)
        .run_jobs_in_background(|_error| {})
        .build()
        .await;

    assert!(
        matches!(&built, Err(IamError::Settings(error)) if error.to_string() == "the settings could not write"),
        "{built:?}"
    );
}

#[test]
fn a_setting_is_named_with_the_prefix_of_the_module() {
    assert_eq!(
        IamSetting::UserDeletionWindow.as_ref(),
        "iam.user_deletion_window"
    );
    assert_eq!(
        IamSetting::TenantDeletionWindow.as_ref(),
        "iam.tenant_deletion_window"
    );
}
