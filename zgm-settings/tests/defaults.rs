//! The defaults the modules give to [`Settings`] when they start: written to the database
//! once, and after that left as they are, whatever a later version of a module gives.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use zgm_settings::{DefaultSetting, Settings, SettingsError, Value};

use common::{IamSetting, NOWAK, days, default_by_hand, default_row};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// The name of the setting most tests give.
const WINDOW: IamSetting = IamSetting::UserDeletionWindow;

fn window(value: Duration, min: Option<Duration>, max: Option<Duration>) -> DefaultSetting {
    DefaultSetting::Duration { value, min, max }
}

/// A row of the defaults: the kind, the JSON as the database writes it, and the bounds.
fn row(
    kind: &str,
    json: &str,
    min: Option<i64>,
    max: Option<i64>,
) -> Option<(String, String, Option<i64>, Option<i64>)> {
    Some((kind.to_owned(), json.to_owned(), min, max))
}

/// The module on a migrated database, with no defaults yet.
async fn settings_on(pool: &PgPool) -> Settings {
    zgm_settings::migrate(pool).await.unwrap();
    Settings::builder().pool(pool.clone()).build()
}

/// Whether the row of a default was changed after it was written.
async fn changed_after_written(pool: &PgPool, name: impl AsRef<str>) -> bool {
    sqlx::query_scalar(
        "SELECT updated_at > created_at FROM zgm_settings.default_settings WHERE name = $1",
    )
    .bind(name.as_ref())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Settings on a pool that never connects.
fn settings_without_a_database() -> Settings {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(500))
        .connect_lazy("postgres://postgres:postgres@127.0.0.1:1/nothing")
        .unwrap();
    Settings::builder().pool(pool).build()
}

#[tokio::test]
async fn new_settings_have_no_defaults() {
    assert_eq!(settings_without_a_database().defaults(), BTreeMap::new());
}

#[sqlx::test(migrations = false)]
async fn the_first_default_of_a_setting_is_written_with_its_kind_and_its_bounds(pool: PgPool) {
    let settings = settings_on(&pool).await;

    let written = [
        settings
            .add_default(WINDOW, window(DAY * 30, Some(DAY), Some(DAY * 365)))
            .await
            .unwrap(),
        settings
            .add_default(
                IamSetting::MaxUsers,
                DefaultSetting::Number {
                    value: 10,
                    min: Some(1),
                    max: Some(1000),
                },
            )
            .await
            .unwrap(),
        settings
            .add_default(IamSetting::SelfSignUp, DefaultSetting::Flag(false))
            .await
            .unwrap(),
        settings
            .add_default(
                IamSetting::WelcomeText,
                DefaultSetting::Text("Welcome".to_owned()),
            )
            .await
            .unwrap(),
    ];

    assert_eq!(written, [true; 4]);
    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "2592000", Some(86_400), Some(31_536_000))
    );
    assert_eq!(
        default_row(&pool, IamSetting::MaxUsers).await,
        row("number", "10", Some(1), Some(1000))
    );
    assert_eq!(
        default_row(&pool, IamSetting::SelfSignUp).await,
        row("flag", "false", None, None)
    );
    assert_eq!(
        default_row(&pool, IamSetting::WelcomeText).await,
        row("text", "\"Welcome\"", None, None)
    );
    assert_eq!(
        settings.defaults(),
        BTreeMap::from([
            ("iam.max_users".to_owned(), Value::Number(10)),
            ("iam.self_sign_up".to_owned(), Value::Flag(false)),
            ("iam.user_deletion_window".to_owned(), days(30)),
            (
                "iam.welcome_text".to_owned(),
                Value::Text("Welcome".to_owned())
            ),
        ])
    );
}

#[sqlx::test(migrations = false)]
async fn a_default_given_again_leaves_the_row_as_it_was(pool: PgPool) {
    let settings = settings_on(&pool).await;
    settings
        .add_default(WINDOW, window(DAY * 30, Some(DAY), Some(DAY * 365)))
        .await
        .unwrap();

    let written = settings
        .add_default(WINDOW, window(DAY * 14, None, Some(DAY * 14)))
        .await
        .unwrap();

    assert!(!written);
    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "2592000", Some(86_400), Some(31_536_000))
    );
    assert!(!changed_after_written(&pool, WINDOW).await);
    assert_eq!(
        settings.defaults(),
        BTreeMap::from([("iam.user_deletion_window".to_owned(), days(30))])
    );
}

#[sqlx::test(migrations = false)]
async fn the_next_version_of_a_module_gets_the_default_the_database_has(pool: PgPool) {
    let first_version = settings_on(&pool).await;
    first_version
        .add_default(WINDOW, window(DAY * 30, Some(DAY), Some(DAY * 365)))
        .await
        .unwrap();

    // The program started again, with another number in the code of the module.
    let next_version = Settings::builder().pool(pool.clone()).build();
    let written = next_version
        .add_default(WINDOW, window(DAY * 14, None, None))
        .await
        .unwrap();

    assert!(!written);
    assert_eq!(
        next_version.defaults(),
        BTreeMap::from([("iam.user_deletion_window".to_owned(), days(30))])
    );
    assert_eq!(
        next_version.tenant_id(NOWAK).get(WINDOW).await.unwrap(),
        days(30)
    );
}

#[sqlx::test(migrations = false)]
async fn a_default_changed_in_the_database_is_the_one_the_module_gets(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    // 90 days, as a change after the first start would leave it.
    default_by_hand(&pool, WINDOW, "duration", "7776000").await;
    let settings = Settings::builder().pool(pool.clone()).build();

    let written = settings
        .add_default(WINDOW, window(DAY * 30, None, None))
        .await
        .unwrap();

    assert!(!written);
    assert_eq!(
        settings.defaults(),
        BTreeMap::from([("iam.user_deletion_window".to_owned(), days(90))])
    );
}

#[sqlx::test(migrations = false)]
async fn a_default_of_another_kind_given_again_leaves_the_row_of_the_first_kind(pool: PgPool) {
    let settings = settings_on(&pool).await;
    settings
        .add_default(WINDOW, window(DAY * 30, None, None))
        .await
        .unwrap();

    let written = settings
        .add_default(
            WINDOW,
            DefaultSetting::Number {
                value: 30,
                min: None,
                max: None,
            },
        )
        .await
        .unwrap();

    assert!(!written);
    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "2592000", None, None)
    );
    assert_eq!(
        settings.defaults(),
        BTreeMap::from([("iam.user_deletion_window".to_owned(), days(30))])
    );
}

#[sqlx::test(migrations = false)]
async fn a_missing_bound_is_written_as_no_bound(pool: PgPool) {
    let settings = settings_on(&pool).await;

    settings
        .add_default(WINDOW, window(DAY * 30, None, Some(DAY)))
        .await
        .unwrap();
    settings
        .add_default(
            IamSetting::MaxUsers,
            DefaultSetting::Number {
                value: 10,
                min: Some(1),
                max: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "2592000", None, Some(86_400))
    );
    assert_eq!(
        default_row(&pool, IamSetting::MaxUsers).await,
        row("number", "10", Some(1), None)
    );
}

#[sqlx::test(migrations = false)]
async fn nothing_given_is_checked(pool: PgPool) {
    let settings = settings_on(&pool).await;

    // A least above the most, and a value outside its own bounds.
    settings
        .add_default(WINDOW, window(DAY * 30, Some(DAY * 365), Some(DAY)))
        .await
        .unwrap();
    settings
        .add_default(
            IamSetting::MaxUsers,
            DefaultSetting::Number {
                value: 0,
                min: Some(10),
                max: Some(5),
            },
        )
        .await
        .unwrap();

    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "2592000", Some(31_536_000), Some(86_400))
    );
    assert_eq!(
        default_row(&pool, IamSetting::MaxUsers).await,
        row("number", "0", Some(10), Some(5))
    );
}

#[sqlx::test(migrations = false)]
async fn a_fraction_of_a_second_is_cut_off(pool: PgPool) {
    let settings = settings_on(&pool).await;

    settings
        .add_default(
            WINDOW,
            window(
                Duration::from_millis(1500),
                Some(Duration::from_millis(999)),
                Some(Duration::from_nanos(2_999_999_999)),
            ),
        )
        .await
        .unwrap();

    assert_eq!(
        default_row(&pool, WINDOW).await,
        row("duration", "1", Some(0), Some(2))
    );
    assert_eq!(
        settings.defaults(),
        BTreeMap::from([(
            "iam.user_deletion_window".to_owned(),
            Value::Duration(Duration::from_secs(1))
        )])
    );
}

#[sqlx::test(migrations = false)]
async fn a_bound_of_more_seconds_than_a_bigint_holds_is_written_as_the_most_it_holds(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let the_most = Duration::from_secs(i64::MAX as u64);

    settings
        .add_default(
            WINDOW,
            window(
                Duration::MAX,
                Some(the_most),
                Some(the_most + Duration::from_secs(1)),
            ),
        )
        .await
        .unwrap();
    settings
        .add_default(
            "iam.longest",
            window(Duration::ZERO, Some(Duration::ZERO), Some(Duration::MAX)),
        )
        .await
        .unwrap();

    // The value is JSON, which holds any number of seconds.
    assert_eq!(
        default_row(&pool, WINDOW).await,
        row(
            "duration",
            "18446744073709551615",
            Some(i64::MAX),
            Some(i64::MAX)
        )
    );
    assert_eq!(
        default_row(&pool, "iam.longest").await,
        row("duration", "0", Some(0), Some(i64::MAX))
    );
    assert_eq!(
        settings.defaults()["iam.user_deletion_window"],
        Value::Duration(Duration::from_secs(u64::MAX))
    );
}

#[sqlx::test(migrations = false)]
async fn the_least_and_the_greatest_number_are_kept(pool: PgPool) {
    let settings = settings_on(&pool).await;

    settings
        .add_default(
            "iam.least",
            DefaultSetting::Number {
                value: i64::MIN,
                min: Some(i64::MIN),
                max: Some(i64::MAX),
            },
        )
        .await
        .unwrap();
    settings
        .add_default(
            "iam.greatest",
            DefaultSetting::Number {
                value: i64::MAX,
                min: None,
                max: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(
        default_row(&pool, "iam.least").await,
        row(
            "number",
            "-9223372036854775808",
            Some(i64::MIN),
            Some(i64::MAX)
        )
    );
    assert_eq!(
        settings.defaults(),
        BTreeMap::from([
            ("iam.greatest".to_owned(), Value::Number(i64::MAX)),
            ("iam.least".to_owned(), Value::Number(i64::MIN)),
        ])
    );
}

#[sqlx::test(migrations = false)]
async fn a_text_is_kept_exactly_as_it_was_given(pool: PgPool) {
    let settings = settings_on(&pool).await;
    let texts = [
        "",
        "Witamy w firmie \"Nowak\"",
        "a \\ backslash",
        "two\nlines\tand a tab",
        "zażółć gęślą jaźń",
        "👋🏽",
        "  spaces round it  ",
        "'; DROP TABLE zgm_settings.default_settings; --",
        "{\"not\": \"json\"}",
        "null",
    ];

    for (number, text) in texts.iter().enumerate() {
        settings
            .add_default(
                format!("iam.text_{number}"),
                DefaultSetting::Text((*text).to_owned()),
            )
            .await
            .unwrap();
    }

    for (number, text) in texts.iter().enumerate() {
        assert_eq!(
            settings.defaults()[&format!("iam.text_{number}")],
            Value::Text((*text).to_owned()),
            "{text}"
        );
    }
}

#[sqlx::test(migrations = false)]
async fn a_text_with_a_nul_character_is_an_error_and_nothing_is_kept(pool: PgPool) {
    let settings = settings_on(&pool).await;

    // PostgreSQL keeps no NUL in a text.
    let refused = settings
        .add_default(
            IamSetting::WelcomeText,
            DefaultSetting::Text("a\0b".to_owned()),
        )
        .await
        .unwrap_err();

    assert!(matches!(refused, SettingsError::Database(_)), "{refused}");
    assert_eq!(default_row(&pool, IamSetting::WelcomeText).await, None);
    assert_eq!(settings.defaults(), BTreeMap::new());
}

#[sqlx::test(migrations = false)]
async fn a_name_is_kept_exactly_as_it_was_given(pool: PgPool) {
    let settings = settings_on(&pool).await;

    for name in [
        "iam.user_deletion_window",
        "billing.Invoice Footer",
        "płatności.stopka",
        "",
    ] {
        settings
            .add_default(name, DefaultSetting::Flag(true))
            .await
            .unwrap();
        assert_eq!(
            default_row(&pool, name).await,
            row("flag", "true", None, None),
            "{name}"
        );
    }
}

#[sqlx::test(migrations = false)]
async fn the_defaults_of_several_modules_are_all_there_in_the_order_of_their_names(pool: PgPool) {
    let settings = settings_on(&pool).await;

    for name in [
        "iam.user_deletion_window",
        "login.session_lifetime",
        "iam.tenant_deletion_window",
        "audit.enabled",
    ] {
        settings
            .add_default(name, DefaultSetting::Flag(false))
            .await
            .unwrap();
    }

    let names: Vec<String> = settings.defaults().into_keys().collect();
    assert_eq!(
        names,
        [
            "audit.enabled",
            "iam.tenant_deletion_window",
            "iam.user_deletion_window",
            "login.session_lifetime",
        ]
    );
}

#[sqlx::test(migrations = false)]
async fn a_clone_sees_the_defaults_given_through_another_clone(pool: PgPool) {
    // The program keeps one, and a module such as IAM gets a clone and gives its defaults
    // through it.
    let of_the_program = settings_on(&pool).await;
    let of_iam = of_the_program.clone();

    of_iam
        .add_default(WINDOW, window(DAY * 30, None, None))
        .await
        .unwrap();

    assert_eq!(
        of_the_program.defaults(),
        BTreeMap::from([("iam.user_deletion_window".to_owned(), days(30))])
    );
}

#[sqlx::test(migrations = false)]
async fn two_copies_of_the_program_giving_a_default_at_once_write_it_once(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    // Two copies of the program, each with a pool of its own.
    let mut copies = Vec::new();
    for _ in 0..2 {
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with((*pool.connect_options()).clone())
            .await
            .unwrap();
        copies.push(Settings::builder().pool(pool).build());
    }

    let (first, second) = tokio::join!(
        copies[0].add_default(WINDOW, window(DAY * 30, None, None)),
        copies[1].add_default(WINDOW, window(DAY * 14, None, None)),
    );

    let (first, second) = (first.unwrap(), second.unwrap());
    assert_ne!(first, second, "exactly one of them writes the row");
    let in_the_database = if first { days(30) } else { days(14) };
    for copy in &copies {
        assert_eq!(
            copy.defaults(),
            BTreeMap::from([(
                "iam.user_deletion_window".to_owned(),
                in_the_database.clone()
            )])
        );
    }
}

#[tokio::test]
async fn when_the_database_does_not_answer_it_is_an_error_and_nothing_is_kept() {
    let settings = settings_without_a_database();

    let refused = settings
        .add_default(WINDOW, window(DAY * 30, None, None))
        .await
        .unwrap_err();

    assert!(matches!(refused, SettingsError::Database(_)), "{refused}");
    assert_eq!(settings.defaults(), BTreeMap::new());
}

#[sqlx::test(migrations = false)]
async fn without_the_migration_it_is_an_error_and_nothing_is_kept(pool: PgPool) {
    let settings = Settings::builder().pool(pool.clone()).build();

    let refused = settings
        .add_default(WINDOW, window(DAY * 30, None, None))
        .await
        .unwrap_err();

    assert!(matches!(refused, SettingsError::Database(_)), "{refused}");
    assert_eq!(settings.defaults(), BTreeMap::new());
}

#[sqlx::test(migrations = false)]
async fn a_row_of_a_default_the_module_cannot_read_is_an_error_and_stays(pool: PgPool) {
    zgm_settings::migrate(&pool).await.unwrap();
    // Rows only SQL by hand can write.
    let rows = [
        (
            "iam.colour",
            "colour",
            "\"blue\"",
            DefaultSetting::Flag(true),
        ),
        (
            "iam.thirty",
            "duration",
            "\"thirty days\"",
            window(DAY * 30, None, None),
        ),
        (
            "iam.backwards",
            "duration",
            "-5",
            window(DAY * 30, None, None),
        ),
        (
            "iam.a_fraction",
            "duration",
            "1.5",
            window(DAY * 30, None, None),
        ),
        ("iam.nothing", "flag", "null", DefaultSetting::Flag(true)),
        (
            "iam.a_count",
            "number",
            "true",
            DefaultSetting::Number {
                value: 1,
                min: None,
                max: None,
            },
        ),
    ];
    for (name, kind, json, _) in &rows {
        default_by_hand(&pool, name, kind, json).await;
    }
    let settings = Settings::builder().pool(pool.clone()).build();

    for (name, kind, json, default) in rows {
        let refused = settings.add_default(name, default).await.unwrap_err();
        assert!(
            matches!(
                &refused,
                SettingsError::UnreadableDefault { name: in_error, kind: kind_in_error, found }
                    if in_error == name && kind_in_error == kind && found == json
            ),
            "{name}: {refused}"
        );
        assert_eq!(
            default_row(&pool, name).await.map(|(_, json, _, _)| json),
            Some(json.to_owned()),
            "{name}"
        );
    }
    assert_eq!(settings.defaults(), BTreeMap::new());
}
