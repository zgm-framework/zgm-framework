//! The defaults the modules give to [`Settings`] when they are built. Nothing here reaches the
//! database.

use std::collections::BTreeMap;
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use zgm_settings::{Settings, Value};

/// The settings of a module, named the way the modules of the framework name them: the
/// prefix of the module, then the variant in snake case.
#[derive(Clone, Copy, strum::AsRefStr)]
#[strum(serialize_all = "snake_case", prefix = "iam.")]
enum IamSetting {
    UserDeletionWindow,
    TenantDeletionWindow,
}

const DAY: u64 = 24 * 60 * 60;

fn days(count: u64) -> Value {
    Value::Duration(Duration::from_secs(count * DAY))
}

/// Settings on a pool that never connects.
fn settings() -> Settings {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://localhost/nothing")
        .unwrap();
    Settings::builder().pool(pool).build()
}

#[tokio::test]
async fn new_settings_have_no_defaults() {
    assert_eq!(settings().defaults(), BTreeMap::new());
}

#[tokio::test]
async fn a_default_of_every_kind_is_kept_as_it_was_given() {
    let settings = settings();

    settings.add_default("iam.a_duration", days(30));
    settings.add_default("iam.a_number", Value::Number(-5));
    settings.add_default("iam.a_flag", Value::Flag(true));
    settings.add_default("iam.a_text", Value::Text("Kowalski".to_owned()));

    assert_eq!(
        settings.defaults(),
        BTreeMap::from([
            ("iam.a_duration".to_owned(), days(30)),
            ("iam.a_flag".to_owned(), Value::Flag(true)),
            ("iam.a_number".to_owned(), Value::Number(-5)),
            ("iam.a_text".to_owned(), Value::Text("Kowalski".to_owned())),
        ])
    );
}

#[tokio::test]
async fn a_setting_of_an_enum_is_named_with_the_prefix_of_its_module() {
    let settings = settings();

    settings.add_default(IamSetting::UserDeletionWindow, days(30));

    assert_eq!(
        settings.defaults(),
        BTreeMap::from([("iam.user_deletion_window".to_owned(), days(30))])
    );
}

#[tokio::test]
async fn a_name_given_as_plain_text_is_kept_exactly_as_it_was_given() {
    let settings = settings();

    settings.add_default(
        "billing.Invoice Footer",
        Value::Text("Thank you".to_owned()),
    );

    assert_eq!(
        settings.defaults(),
        BTreeMap::from([(
            "billing.Invoice Footer".to_owned(),
            Value::Text("Thank you".to_owned())
        )])
    );
}

#[tokio::test]
async fn a_second_default_of_the_same_name_replaces_the_first() {
    let settings = settings();

    settings.add_default(IamSetting::UserDeletionWindow, days(30));
    settings.add_default(IamSetting::UserDeletionWindow, days(14));

    assert_eq!(
        settings.defaults(),
        BTreeMap::from([("iam.user_deletion_window".to_owned(), days(14))])
    );
}

#[tokio::test]
async fn the_defaults_of_several_modules_are_all_there_in_the_order_of_their_names() {
    let settings = settings();

    settings.add_default(IamSetting::UserDeletionWindow, days(30));
    settings.add_default("login.session_lifetime", days(7));
    settings.add_default(IamSetting::TenantDeletionWindow, days(90));
    settings.add_default("audit.enabled", Value::Flag(false));

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

#[tokio::test]
async fn a_clone_sees_the_defaults_given_through_another_clone() {
    // The program keeps one, and a module such as IAM gets a clone and gives its defaults
    // through it.
    let of_the_program = settings();
    let of_iam = of_the_program.clone();

    of_iam.add_default(IamSetting::UserDeletionWindow, days(30));

    assert_eq!(
        of_the_program.defaults(),
        BTreeMap::from([("iam.user_deletion_window".to_owned(), days(30))])
    );
}
