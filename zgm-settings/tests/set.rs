//! `set`: a tenant changes the value of a setting.

mod common;

use std::time::Duration;

use sqlx::PgPool;
use zgm_settings::{SettingsError, Value};

use common::{
    IamSetting, KOWALSKI, NOWAK, changed_after_written, days, json_of, rows, set_by_hand,
    settings_on,
};

#[sqlx::test(migrations = false)]
async fn a_value_set_of_every_kind_is_what_get_gives(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    for (setting, value) in [
        (IamSetting::UserDeletionWindow, days(90)),
        (IamSetting::UserDeletionWindow, days(0)),
        (
            IamSetting::UserDeletionWindow,
            Value::Duration(Duration::from_secs(u64::MAX)),
        ),
        (IamSetting::MaxUsers, Value::Number(250)),
        (IamSetting::MaxUsers, Value::Number(i64::MIN)),
        (IamSetting::MaxUsers, Value::Number(i64::MAX)),
        (IamSetting::SelfSignUp, Value::Flag(true)),
        (
            IamSetting::WelcomeText,
            Value::Text("Firma \"Nowak\" — Łódź, zażółć\\gęślą\njaźń".to_owned()),
        ),
        (IamSetting::WelcomeText, Value::Text(String::new())),
    ] {
        in_nowak.set(setting, value.clone()).await.unwrap();
        assert_eq!(in_nowak.get(setting).await.unwrap(), value, "{value:?}");
    }
}

#[sqlx::test(migrations = false)]
async fn a_value_is_written_as_plain_json(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    in_nowak
        .set(IamSetting::UserDeletionWindow, days(90))
        .await
        .unwrap();
    in_nowak
        .set(IamSetting::MaxUsers, Value::Number(-5))
        .await
        .unwrap();
    in_nowak
        .set(IamSetting::SelfSignUp, Value::Flag(true))
        .await
        .unwrap();
    in_nowak
        .set(IamSetting::WelcomeText, Value::Text("a \"b\"".to_owned()))
        .await
        .unwrap();

    assert_eq!(
        json_of(&pool, NOWAK, IamSetting::UserDeletionWindow)
            .await
            .as_deref(),
        Some("7776000")
    );
    assert_eq!(
        json_of(&pool, NOWAK, IamSetting::MaxUsers).await.as_deref(),
        Some("-5")
    );
    assert_eq!(
        json_of(&pool, NOWAK, IamSetting::SelfSignUp)
            .await
            .as_deref(),
        Some("true")
    );
    assert_eq!(
        json_of(&pool, NOWAK, IamSetting::WelcomeText)
            .await
            .as_deref(),
        Some(r#""a \"b\"""#)
    );
}

#[sqlx::test(migrations = false)]
async fn a_new_value_is_a_change(pool: PgPool) {
    let settings = settings_on(&pool).await;

    let changed = settings
        .tenant_id(NOWAK)
        .set(IamSetting::UserDeletionWindow, days(90))
        .await
        .unwrap();

    assert!(changed);
    assert_eq!(rows(&pool).await, 1);
}

#[sqlx::test(migrations = false)]
async fn another_value_replaces_the_one_before_and_moves_updated_at(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    in_nowak
        .set(IamSetting::UserDeletionWindow, days(90))
        .await
        .unwrap();

    let changed = in_nowak
        .set(IamSetting::UserDeletionWindow, days(14))
        .await
        .unwrap();

    assert!(changed);
    assert_eq!(
        in_nowak.get(IamSetting::UserDeletionWindow).await.unwrap(),
        days(14)
    );
    assert_eq!(rows(&pool).await, 1);
    assert!(changed_after_written(&pool, NOWAK, IamSetting::UserDeletionWindow).await);
}

#[sqlx::test(migrations = false)]
async fn the_same_value_again_changes_nothing_not_even_updated_at(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    in_nowak
        .set(IamSetting::UserDeletionWindow, days(90))
        .await
        .unwrap();

    let changed = in_nowak
        .set(IamSetting::UserDeletionWindow, days(90))
        .await
        .unwrap();

    assert!(!changed);
    assert!(!changed_after_written(&pool, NOWAK, IamSetting::UserDeletionWindow).await);
}

#[sqlx::test(migrations = false)]
async fn the_value_of_another_tenant_stays_as_it_was(pool: PgPool) {
    let settings = settings_on(&pool).await;

    settings
        .tenant_id(NOWAK)
        .set(IamSetting::UserDeletionWindow, days(90))
        .await
        .unwrap();
    settings
        .tenant_id(KOWALSKI)
        .set(IamSetting::UserDeletionWindow, days(14))
        .await
        .unwrap();

    assert_eq!(
        settings
            .tenant_id(NOWAK)
            .get(IamSetting::UserDeletionWindow)
            .await
            .unwrap(),
        days(90)
    );
    assert_eq!(rows(&pool).await, 2);
}

#[sqlx::test(migrations = false)]
async fn a_value_equal_to_the_default_is_kept_as_a_row(pool: PgPool) {
    let settings = settings_on(&pool).await;

    let changed = settings
        .tenant_id(NOWAK)
        .set(IamSetting::UserDeletionWindow, days(30))
        .await
        .unwrap();

    assert!(changed);
    assert_eq!(
        json_of(&pool, NOWAK, IamSetting::UserDeletionWindow)
            .await
            .as_deref(),
        Some("2592000")
    );
}

#[sqlx::test(migrations = false)]
async fn a_name_without_a_default_is_refused_and_nothing_is_written(pool: PgPool) {
    let settings = settings_on(&pool).await;

    let refused = settings
        .tenant_id(NOWAK)
        .set("iam.unknown", days(90))
        .await
        .unwrap_err();

    assert!(
        matches!(&refused, SettingsError::UnknownSetting(name) if name == "iam.unknown"),
        "{refused}"
    );
    assert_eq!(rows(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_value_of_another_kind_than_the_default_is_refused_and_nothing_changes(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    // Every setting has a value of its own already, which must stay.
    for (setting, json) in [
        (IamSetting::UserDeletionWindow, "7776000"),
        (IamSetting::MaxUsers, "250"),
        (IamSetting::SelfSignUp, "true"),
        (IamSetting::WelcomeText, r#""Witamy""#),
    ] {
        set_by_hand(&pool, NOWAK, setting, json).await;
    }

    for (setting, value, kept) in [
        (IamSetting::UserDeletionWindow, Value::Number(90), "7776000"),
        (
            IamSetting::UserDeletionWindow,
            Value::Text("90 days".to_owned()),
            "7776000",
        ),
        (IamSetting::MaxUsers, days(1), "250"),
        (IamSetting::MaxUsers, Value::Flag(true), "250"),
        (IamSetting::SelfSignUp, Value::Number(1), "true"),
        (
            IamSetting::SelfSignUp,
            Value::Text("true".to_owned()),
            "true",
        ),
        (IamSetting::WelcomeText, Value::Number(5), r#""Witamy""#),
        (IamSetting::WelcomeText, Value::Flag(false), r#""Witamy""#),
    ] {
        let refused = in_nowak.set(setting, value.clone()).await.unwrap_err();
        assert!(
            matches!(
                &refused,
                SettingsError::WrongKind { name, found }
                    if name == setting.as_ref() && found == &format!("{value:?}")
            ),
            "{value:?}: {refused}"
        );
        assert_eq!(
            json_of(&pool, NOWAK, setting).await.as_deref(),
            Some(kept),
            "{value:?}"
        );
    }
    assert_eq!(rows(&pool).await, 4);
}

#[sqlx::test(migrations = false)]
async fn a_duration_with_a_fraction_of_a_second_is_refused_and_nothing_is_written(pool: PgPool) {
    let settings = settings_on(&pool).await;

    for duration in [Duration::from_millis(1500), Duration::from_nanos(1)] {
        let refused = settings
            .tenant_id(NOWAK)
            .set(IamSetting::UserDeletionWindow, Value::Duration(duration))
            .await
            .unwrap_err();
        assert!(
            matches!(
                &refused,
                SettingsError::FractionOfASecond(name) if name == "iam.user_deletion_window"
            ),
            "{duration:?}: {refused}"
        );
    }
    assert_eq!(rows(&pool).await, 0);
}
