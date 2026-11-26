//! `restore_default`: a tenant goes back to the default of a setting, by losing its row.

mod common;

use sqlx::PgPool;
use zgm_settings::SettingsError;

use common::{
    IamSetting, KOWALSKI, NOWAK, days, default_by_hand, json_of, rows, set_by_hand, settings_on,
};

#[sqlx::test(migrations = false)]
async fn the_row_goes_and_the_tenant_has_the_default_again(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;

    let restored = in_nowak
        .restore_default(IamSetting::UserDeletionWindow)
        .await
        .unwrap();

    assert!(restored);
    assert_eq!(
        json_of(&pool, NOWAK, IamSetting::UserDeletionWindow).await,
        None
    );
    assert_eq!(
        in_nowak.get(IamSetting::UserDeletionWindow).await.unwrap(),
        days(30)
    );
}

#[sqlx::test(migrations = false)]
async fn without_a_row_there_is_nothing_to_restore(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;
    in_nowak
        .restore_default(IamSetting::UserDeletionWindow)
        .await
        .unwrap();

    let restored = in_nowak
        .restore_default(IamSetting::UserDeletionWindow)
        .await
        .unwrap();

    assert!(!restored);
}

#[sqlx::test(migrations = false)]
async fn the_row_of_another_tenant_and_another_setting_stay(pool: PgPool) {
    let settings = settings_on(&pool).await;
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;
    set_by_hand(&pool, NOWAK, IamSetting::MaxUsers, "250").await;
    set_by_hand(&pool, KOWALSKI, IamSetting::UserDeletionWindow, "1209600").await;

    settings
        .tenant_id(NOWAK)
        .restore_default(IamSetting::UserDeletionWindow)
        .await
        .unwrap();

    assert_eq!(rows(&pool).await, 2);
    assert_eq!(
        json_of(&pool, NOWAK, IamSetting::MaxUsers).await.as_deref(),
        Some("250")
    );
    assert_eq!(
        json_of(&pool, KOWALSKI, IamSetting::UserDeletionWindow)
            .await
            .as_deref(),
        Some("1209600")
    );
}

#[sqlx::test(migrations = false)]
async fn a_name_without_a_default_is_refused_and_its_row_stays(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // A default an earlier version of the module left, which this one no longer gives.
    default_by_hand(&pool, "iam.dropped_setting", "duration", "2592000").await;
    set_by_hand(&pool, NOWAK, "iam.dropped_setting", "7776000").await;

    let refused = settings
        .tenant_id(NOWAK)
        .restore_default("iam.dropped_setting")
        .await
        .unwrap_err();

    assert!(
        matches!(&refused, SettingsError::UnknownSetting(name) if name == "iam.dropped_setting"),
        "{refused}"
    );
    assert_eq!(
        json_of(&pool, NOWAK, "iam.dropped_setting")
            .await
            .as_deref(),
        Some("7776000")
    );
}

#[sqlx::test(migrations = false)]
async fn a_row_of_another_kind_than_the_default_goes_too(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    // Only SQL by hand can write it; `get` refuses it.
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, r#""90 days""#).await;

    let restored = in_nowak
        .restore_default(IamSetting::UserDeletionWindow)
        .await
        .unwrap();

    assert!(restored);
    assert_eq!(
        in_nowak.get(IamSetting::UserDeletionWindow).await.unwrap(),
        days(30)
    );
}
