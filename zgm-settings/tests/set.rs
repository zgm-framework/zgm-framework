//! `set`: a tenant changes the value of a setting, of the kind and within the bounds the row
//! of its default has in the database.

mod common;

use std::time::Duration;

use sqlx::PgPool;
use zgm_settings::{DefaultSetting, Settings, SettingsError, Value};

use common::{
    IamSetting, KOWALSKI, NOWAK, changed_after_written, days, default_by_hand, json_of, rows,
    set_by_hand, settings_on,
};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// The module on a migrated database, with the defaults of [`IamSetting`] and bounds: the
/// window 30 days, at least a day and at most a year; at most 10 users by default, at least 1
/// and at most 1000; no for self sign-up and "Welcome", both without bounds.
async fn settings_with_bounds_on(pool: &PgPool) -> Settings {
    zgm_settings::migrate(pool).await.unwrap();
    let settings = Settings::builder().pool(pool.clone()).build();
    for (setting, default) in [
        (
            IamSetting::UserDeletionWindow,
            DefaultSetting::Duration {
                value: DAY * 30,
                min: Some(DAY),
                max: Some(DAY * 365),
            },
        ),
        (
            IamSetting::MaxUsers,
            DefaultSetting::Number {
                value: 10,
                min: Some(1),
                max: Some(1000),
            },
        ),
        (IamSetting::SelfSignUp, DefaultSetting::Flag(false)),
        (
            IamSetting::WelcomeText,
            DefaultSetting::Text("Welcome".to_owned()),
        ),
    ] {
        settings.add_default(setting, default).await.unwrap();
    }
    settings
}

/// Changes the bounds of a default with SQL by hand, as the function that changes a default
/// will.
async fn bounds_by_hand(pool: &PgPool, name: impl AsRef<str>, min: Option<i64>, max: Option<i64>) {
    sqlx::query("UPDATE zgm_settings.default_settings SET min = $2, max = $3 WHERE name = $1")
        .bind(name.as_ref())
        .bind(min)
        .bind(max)
        .execute(pool)
        .await
        .unwrap();
}

fn below_the_least(refused: &SettingsError, setting: IamSetting, least: &Value) -> bool {
    matches!(
        refused,
        SettingsError::BelowTheLeast { name, min } if name == setting.as_ref() && min == least
    )
}

fn above_the_most(refused: &SettingsError, setting: IamSetting, most: &Value) -> bool {
    matches!(
        refused,
        SettingsError::AboveTheMost { name, max } if name == setting.as_ref() && max == most
    )
}

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

#[sqlx::test(migrations = false)]
async fn a_value_within_the_bounds_is_set_the_bounds_themselves_too(pool: PgPool) {
    let settings = settings_with_bounds_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    for (setting, value) in [
        (IamSetting::UserDeletionWindow, days(90)),
        (IamSetting::UserDeletionWindow, days(1)),
        (IamSetting::UserDeletionWindow, days(365)),
        (IamSetting::MaxUsers, Value::Number(1)),
        (IamSetting::MaxUsers, Value::Number(1000)),
    ] {
        assert!(
            in_nowak.set(setting, value.clone()).await.unwrap(),
            "{value:?}"
        );
        assert_eq!(in_nowak.get(setting).await.unwrap(), value);
    }
}

#[sqlx::test(migrations = false)]
async fn a_value_below_the_least_is_refused_and_nothing_changes(pool: PgPool) {
    let settings = settings_with_bounds_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;

    for (setting, value, least) in [
        (IamSetting::UserDeletionWindow, days(0), days(1)),
        (
            IamSetting::UserDeletionWindow,
            Value::Duration(DAY - Duration::from_secs(1)),
            days(1),
        ),
        (IamSetting::MaxUsers, Value::Number(0), Value::Number(1)),
        (
            IamSetting::MaxUsers,
            Value::Number(i64::MIN),
            Value::Number(1),
        ),
    ] {
        let refused = in_nowak.set(setting, value.clone()).await.unwrap_err();
        assert!(
            below_the_least(&refused, setting, &least),
            "{value:?}: {refused}"
        );
    }
    assert_eq!(
        json_of(&pool, NOWAK, IamSetting::UserDeletionWindow)
            .await
            .as_deref(),
        Some("7776000")
    );
    assert_eq!(rows(&pool).await, 1);
}

#[sqlx::test(migrations = false)]
async fn a_value_above_the_most_is_refused_and_nothing_changes(pool: PgPool) {
    let settings = settings_with_bounds_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    set_by_hand(&pool, NOWAK, IamSetting::UserDeletionWindow, "7776000").await;

    for (setting, value, most) in [
        (IamSetting::UserDeletionWindow, days(400), days(365)),
        (
            IamSetting::UserDeletionWindow,
            Value::Duration(DAY * 365 + Duration::from_secs(1)),
            days(365),
        ),
        (
            IamSetting::UserDeletionWindow,
            Value::Duration(Duration::from_secs(u64::MAX)),
            days(365),
        ),
        (
            IamSetting::MaxUsers,
            Value::Number(1001),
            Value::Number(1000),
        ),
        (
            IamSetting::MaxUsers,
            Value::Number(i64::MAX),
            Value::Number(1000),
        ),
    ] {
        let refused = in_nowak.set(setting, value.clone()).await.unwrap_err();
        assert!(
            above_the_most(&refused, setting, &most),
            "{value:?}: {refused}"
        );
    }
    assert_eq!(
        json_of(&pool, NOWAK, IamSetting::UserDeletionWindow)
            .await
            .as_deref(),
        Some("7776000")
    );
    assert_eq!(rows(&pool).await, 1);
}

#[sqlx::test(migrations = false)]
async fn a_bound_that_is_none_holds_nothing(pool: PgPool) {
    let settings = settings_with_bounds_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    // No least: anything up to the most.
    bounds_by_hand(&pool, IamSetting::MaxUsers, None, Some(1000)).await;
    in_nowak
        .set(IamSetting::MaxUsers, Value::Number(i64::MIN))
        .await
        .unwrap();
    // No most: anything from the least.
    bounds_by_hand(&pool, IamSetting::UserDeletionWindow, Some(86_400), None).await;
    in_nowak
        .set(
            IamSetting::UserDeletionWindow,
            Value::Duration(Duration::from_secs(u64::MAX)),
        )
        .await
        .unwrap();

    assert_eq!(
        in_nowak.get(IamSetting::MaxUsers).await.unwrap(),
        Value::Number(i64::MIN)
    );
    assert_eq!(
        in_nowak.get(IamSetting::UserDeletionWindow).await.unwrap(),
        Value::Duration(Duration::from_secs(u64::MAX))
    );
}

#[sqlx::test(migrations = false)]
async fn the_same_value_again_above_a_most_lowered_since_is_refused(pool: PgPool) {
    let settings = settings_with_bounds_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    in_nowak
        .set(IamSetting::UserDeletionWindow, days(90))
        .await
        .unwrap();
    // The most lowered to 60 days.
    bounds_by_hand(
        &pool,
        IamSetting::UserDeletionWindow,
        Some(86_400),
        Some(5_184_000),
    )
    .await;

    let refused = in_nowak
        .set(IamSetting::UserDeletionWindow, days(90))
        .await
        .unwrap_err();

    assert!(
        above_the_most(&refused, IamSetting::UserDeletionWindow, &days(60)),
        "{refused}"
    );
    // Nowak keeps the 90 days it set before.
    assert_eq!(
        in_nowak.get(IamSetting::UserDeletionWindow).await.unwrap(),
        days(90)
    );
}

#[sqlx::test(migrations = false)]
async fn a_change_of_the_bounds_holds_for_the_next_set(pool: PgPool) {
    let settings = settings_with_bounds_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    bounds_by_hand(
        &pool,
        IamSetting::UserDeletionWindow,
        Some(86_400),
        Some(5_184_000),
    )
    .await;
    let refused = in_nowak
        .set(IamSetting::UserDeletionWindow, days(90))
        .await
        .unwrap_err();
    assert!(
        above_the_most(&refused, IamSetting::UserDeletionWindow, &days(60)),
        "{refused}"
    );

    bounds_by_hand(
        &pool,
        IamSetting::UserDeletionWindow,
        Some(86_400),
        Some(31_536_000),
    )
    .await;
    assert!(
        in_nowak
            .set(IamSetting::UserDeletionWindow, days(90))
            .await
            .unwrap()
    );
}

#[sqlx::test(migrations = false)]
async fn a_least_above_the_most_refuses_every_value(pool: PgPool) {
    let settings = settings_with_bounds_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    // Nothing checks the bounds a module gives, so a mistake in them comes to this.
    bounds_by_hand(&pool, IamSetting::MaxUsers, Some(100), Some(10)).await;

    for value in [5, 50, 500] {
        assert!(
            in_nowak
                .set(IamSetting::MaxUsers, Value::Number(value))
                .await
                .is_err(),
            "{value}"
        );
    }
    assert_eq!(rows(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn bounds_on_a_flag_or_a_text_hold_nothing(pool: PgPool) {
    let settings = settings_with_bounds_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);
    // Bounds only SQL by hand can give a flag or a text.
    bounds_by_hand(&pool, IamSetting::SelfSignUp, Some(5), Some(1)).await;
    bounds_by_hand(&pool, IamSetting::WelcomeText, Some(5), Some(1)).await;

    in_nowak
        .set(IamSetting::SelfSignUp, Value::Flag(true))
        .await
        .unwrap();
    in_nowak
        .set(IamSetting::WelcomeText, Value::Text("Witamy".to_owned()))
        .await
        .unwrap();

    assert_eq!(rows(&pool).await, 2);
}

#[sqlx::test(migrations = false)]
async fn the_kind_and_the_bounds_come_from_the_database_not_from_this_program(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    // A default an earlier version of the module left: a number, at most 5. This program gives
    // no default at all.
    default_by_hand(&pool, "iam.max_admins", "number", "3").await;
    bounds_by_hand(&pool, "iam.max_admins", None, Some(5)).await;
    let settings = Settings::builder().pool(pool.clone()).build();
    let in_nowak = settings.tenant_id(NOWAK);

    assert!(
        in_nowak
            .set("iam.max_admins", Value::Number(4))
            .await
            .unwrap()
    );
    assert!(matches!(
        in_nowak
            .set("iam.max_admins", Value::Number(6))
            .await
            .unwrap_err(),
        SettingsError::AboveTheMost { .. }
    ));
    assert!(matches!(
        in_nowak.set("iam.max_admins", days(4)).await.unwrap_err(),
        SettingsError::WrongKind { .. }
    ));
}

#[sqlx::test(migrations = false)]
async fn a_default_the_module_cannot_read_refuses_and_nothing_is_written(pool: PgPool) {
    let settings = settings_on(&pool).await;
    // A row only SQL by hand can write.
    default_by_hand(&pool, "iam.colour", "colour", r#""blue""#).await;

    let refused = settings
        .tenant_id(NOWAK)
        .set("iam.colour", Value::Text("red".to_owned()))
        .await
        .unwrap_err();

    assert!(
        matches!(&refused, SettingsError::UnreadableDefault { name, .. } if name == "iam.colour"),
        "{refused}"
    );
    assert_eq!(rows(&pool).await, 0);
}

#[sqlx::test(migrations = false)]
async fn a_bound_of_a_duration_below_zero_is_held_as_no_time(pool: PgPool) {
    let settings = settings_with_bounds_on(&pool).await;
    let in_nowak = settings.tenant_id(NOWAK);

    // Only SQL by hand can write it. No duration is below a least below zero, and every one
    // is above a most below zero, which the error shows as no time.
    bounds_by_hand(&pool, IamSetting::UserDeletionWindow, Some(-5), None).await;
    assert!(
        in_nowak
            .set(IamSetting::UserDeletionWindow, days(0))
            .await
            .unwrap()
    );

    bounds_by_hand(&pool, IamSetting::UserDeletionWindow, None, Some(-5)).await;
    let refused = in_nowak
        .set(IamSetting::UserDeletionWindow, days(0))
        .await
        .unwrap_err();
    assert!(
        above_the_most(&refused, IamSetting::UserDeletionWindow, &days(0)),
        "{refused}"
    );
}
