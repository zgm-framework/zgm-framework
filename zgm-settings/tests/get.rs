//! `get`: the value of a setting in one tenant, from its row or, without one, the default.

mod common;

use sqlx::{AssertSqlSafe, PgPool};
use zgm_settings::{SettingsError, Value};

use common::{IamSetting, KOWALSKI, NOWAK, days, set_by_hand, settings_on};

#[sqlx::test(migrations = false)]
async fn without_a_row_a_tenant_gets_the_default(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    assert_eq!(
        in_nowak.get(IamSetting::UserDeletionWindow).await.unwrap(),
        days(30)
    );
    assert_eq!(
        in_nowak.get(IamSetting::MaxUsers).await.unwrap(),
        Value::Number(10)
    );
    assert_eq!(
        in_nowak.get(IamSetting::SelfSignUp).await.unwrap(),
        Value::Flag(false)
    );
    assert_eq!(
        in_nowak.get(IamSetting::WelcomeText).await.unwrap(),
        Value::Text("Welcome".to_owned())
    );
}

#[sqlx::test(migrations = false)]
async fn with_a_row_a_tenant_gets_its_value_of_every_kind(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    for (setting, json, value) in [
        (IamSetting::UserDeletionWindow, "7776000", days(90)),
        (IamSetting::UserDeletionWindow, "0", days(0)),
        (IamSetting::MaxUsers, "250", Value::Number(250)),
        (
            IamSetting::MaxUsers,
            "-9223372036854775808",
            Value::Number(i64::MIN),
        ),
        (IamSetting::SelfSignUp, "true", Value::Flag(true)),
        (
            IamSetting::WelcomeText,
            r#""Firma \"Nowak\"""#,
            Value::Text("Firma \"Nowak\"".to_owned()),
        ),
    ] {
        set_by_hand(&pool, NOWAK, setting, json).await;
        assert_eq!(in_nowak.get(setting).await.unwrap(), value, "{json}");
    }
}

#[sqlx::test(migrations = false)]
async fn the_row_of_another_tenant_changes_nothing(pool: PgPool) {
    let settings = settings_on(&pool).await;

    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;

    assert_eq!(
        settings
            .tenant_id(KOWALSKI)
            .get(IamSetting::UserDeletionWindow)
            .await
            .unwrap(),
        days(30)
    );
}

#[sqlx::test(migrations = false)]
async fn a_change_of_the_row_is_seen_by_the_next_read(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;
    assert_eq!(
        in_nowak.get(IamSetting::UserDeletionWindow).await.unwrap(),
        days(90)
    );

    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "1209600").await;
    assert_eq!(
        in_nowak.get(IamSetting::UserDeletionWindow).await.unwrap(),
        days(14)
    );

    sqlx::query("DELETE FROM zgm_settings.tenant_settings")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        in_nowak.get(IamSetting::UserDeletionWindow).await.unwrap(),
        days(30)
    );
}

#[sqlx::test(migrations = false)]
async fn a_name_without_a_default_is_an_error_with_a_row_and_without(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    let without_a_row = in_nowak.get("iam.unknown").await.unwrap_err();
    assert!(
        matches!(&without_a_row, SettingsError::UnknownSetting(name) if name == "iam.unknown"),
        "{without_a_row}"
    );

    set_by_hand(&pool, NOWAK, "iam.unknown", "7776000").await;
    let with_a_row = in_nowak.get("iam.unknown").await.unwrap_err();
    assert!(
        matches!(&with_a_row, SettingsError::UnknownSetting(name) if name == "iam.unknown"),
        "{with_a_row}"
    );
}

#[sqlx::test(migrations = false)]
async fn a_value_of_another_kind_than_the_default_is_an_error(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    for (setting, json) in [
        (IamSetting::UserDeletionWindow, r#""90 days""#),
        (IamSetting::UserDeletionWindow, r#""7776000""#),
        (IamSetting::UserDeletionWindow, "-5"),
        (IamSetting::UserDeletionWindow, "1.5"),
        (IamSetting::UserDeletionWindow, "true"),
        (IamSetting::MaxUsers, "1.5"),
        (IamSetting::MaxUsers, "9223372036854775808"),
        (IamSetting::MaxUsers, r#""5""#),
        (IamSetting::SelfSignUp, "1"),
        (IamSetting::SelfSignUp, r#""true""#),
        (IamSetting::WelcomeText, "5"),
        (IamSetting::WelcomeText, r#"{"text": "Welcome"}"#),
        (IamSetting::WelcomeText, "null"),
    ] {
        set_by_hand(&pool, NOWAK, setting, json).await;
        let refused = in_nowak.get(setting).await.unwrap_err();
        assert!(
            matches!(
                &refused,
                SettingsError::WrongKind { name, found }
                    if name == setting.as_ref() && found == &normalised(&pool, json).await
            ),
            "{json}: {refused}"
        );
    }
}

/// The text of a JSON the way the database writes it back.
async fn normalised(pool: &PgPool, json: &str) -> String {
    sqlx::query_scalar(AssertSqlSafe("SELECT $1::jsonb::text"))
        .bind(json)
        .fetch_one(pool)
        .await
        .unwrap()
}
