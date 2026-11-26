//! `restore_default`: a tenant goes back to the default of a setting, by losing its row. The
//! setting is known when its default is in the database.

mod common;

use sqlx::PgPool;
use zgm_settings::{Settings, SettingsError};

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
async fn a_name_without_a_default_in_the_database_is_refused(pool: PgPool) {
    let settings = settings_on(&pool).await;

    let refused = settings
        .tenant_id(NOWAK)
        .restore_default("iam.unknown")
        .await
        .unwrap_err();

    assert!(
        matches!(&refused, SettingsError::UnknownSetting(name) if name == "iam.unknown"),
        "{refused}"
    );
}

#[sqlx::test(migrations = false)]
async fn a_setting_this_program_did_not_give_is_restored_too(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // A default an earlier version of the module left, which this one no longer gives.
    default_by_hand(&pool, "iam.dropped_setting", "duration", "2592000").await;
    set_by_hand(&pool, NOWAK, "iam.dropped_setting", "7776000").await;

    let restored = settings
        .tenant_id(NOWAK)
        .restore_default("iam.dropped_setting")
        .await
        .unwrap();

    assert!(restored);
    assert_eq!(json_of(&pool, NOWAK, "iam.dropped_setting").await, None);
}

#[sqlx::test(migrations = false)]
async fn a_copy_of_the_program_that_gave_no_default_restores_too(pool: PgPool) {
    settings_on(&pool).await;
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;
    let another_copy = Settings::builder().pool(pool.clone()).build();

    let restored = another_copy
        .tenant_id(NOWAK)
        .restore_default(IamSetting::UserDeletionWindow)
        .await
        .unwrap();

    assert!(restored);
    assert_eq!(rows(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn the_tenant_gets_the_default_the_database_has_now(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;
    // 45 days, as the function that changes a default will write it.
    sqlx::query("UPDATE zgm_settings.default_settings SET value = '3888000' WHERE name = $1")
        .bind(IamSetting::UserDeletionWindow.as_ref())
        .execute(&pool)
        .await
        .unwrap();

    in_nowak
        .restore_default(IamSetting::UserDeletionWindow)
        .await
        .unwrap();

    assert_eq!(
        in_nowak.get(IamSetting::UserDeletionWindow).await.unwrap(),
        days(45)
    );
}

#[sqlx::test(migrations = false)]
async fn the_row_goes_also_when_the_default_cannot_be_read(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // Rows only SQL by hand can write; the default is not read to delete the row.
    default_by_hand(&pool, "iam.colour", "colour", r#""blue""#).await;
    set_by_hand(&pool, NOWAK, "iam.colour", r#""red""#).await;

    let restored = settings
        .tenant_id(NOWAK)
        .restore_default("iam.colour")
        .await
        .unwrap();

    assert!(restored);
    assert_eq!(rows(&pool).await, 0);
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
