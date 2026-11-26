//! `get`: the value of a setting in one tenant, from its row or, without one, the default,
//! both read from the database every time.

mod common;

use sqlx::{AssertSqlSafe, PgPool};
use zgm_settings::{Settings, SettingsError, Value};

use common::{IamSetting, KOWALSKI, NOWAK, days, default_by_hand, set_by_hand, settings_on};

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
async fn a_name_without_a_default_in_the_database_is_an_error(pool: PgPool) {
    let settings = settings_on(&pool).await;

    let refused = settings
        .tenant_id(NOWAK)
        .get("iam.unknown")
        .await
        .unwrap_err();

    assert!(
        matches!(&refused, SettingsError::UnknownSetting(name) if name == "iam.unknown"),
        "{refused}"
    );
}

#[sqlx::test(migrations = false)]
async fn a_setting_this_program_did_not_give_is_read_from_the_database(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // A default an earlier version of the module left, which this one no longer gives.
    default_by_hand(&pool, "iam.dropped_setting", "duration", "2592000").await;
    set_by_hand(&pool, NOWAK, "iam.dropped_setting", "7776000").await;

    assert_eq!(
        settings
            .tenant_id(NOWAK)
            .get("iam.dropped_setting")
            .await
            .unwrap(),
        days(90)
    );
    assert_eq!(
        settings
            .tenant_id(KOWALSKI)
            .get("iam.dropped_setting")
            .await
            .unwrap(),
        days(30)
    );
}

#[sqlx::test(migrations = false)]
async fn another_copy_of_the_program_reads_the_defaults_without_giving_them(pool: PgPool) {
    settings_on(&pool).await;
    // A copy that has given no default, e.g. one that started while another gave them.
    let another_copy = Settings::builder().pool(pool.clone()).build();

    assert_eq!(
        another_copy
            .tenant_id(KOWALSKI)
            .get(IamSetting::UserDeletionWindow)
            .await
            .unwrap(),
        days(30)
    );
}

#[sqlx::test(migrations = false)]
async fn a_change_of_the_default_is_seen_by_the_next_read(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let in_kowalski = settings.tenant_id(KOWALSKI);
    assert_eq!(
        in_kowalski
            .get(IamSetting::UserDeletionWindow)
            .await
            .unwrap(),
        days(30)
    );

    // 45 days, as the function that changes a default will write it.
    sqlx::query("UPDATE zgm_settings.default_settings SET value = '3888000' WHERE name = $1")
        .bind(IamSetting::UserDeletionWindow.as_ref())
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        in_kowalski
            .get(IamSetting::UserDeletionWindow)
            .await
            .unwrap(),
        days(45)
    );
}

#[sqlx::test(migrations = false)]
async fn a_value_above_a_most_lowered_after_it_was_set_stays_the_value_of_the_tenant(pool: PgPool) {
    let settings = settings_on(&pool).await;
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;

    // The most lowered to 60 days after Nowak set 90: it holds for changes from now on.
    sqlx::query("UPDATE zgm_settings.default_settings SET max = 5184000 WHERE name = $1")
        .bind(IamSetting::UserDeletionWindow.as_ref())
        .execute(&pool)
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
}

#[sqlx::test(migrations = false)]
async fn a_default_the_module_cannot_read_is_an_error_with_a_row_and_without(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // A row only SQL by hand can write.
    default_by_hand(&pool, "iam.colour", "colour", r#""blue""#).await;

    let without_a_row = settings
        .tenant_id(KOWALSKI)
        .get("iam.colour")
        .await
        .unwrap_err();
    set_by_hand(&pool, NOWAK, "iam.colour", r#""red""#).await;
    let with_a_row = settings
        .tenant_id(NOWAK)
        .get("iam.colour")
        .await
        .unwrap_err();

    for refused in [without_a_row, with_a_row] {
        assert!(
            matches!(
                &refused,
                SettingsError::UnreadableDefault { name, kind, found }
                    if name == "iam.colour" && kind == "colour" && found == r#""blue""#
            ),
            "{refused}"
        );
    }
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
