//! A tour of `zgm-settings`: one run that uses everything the module offers and prints what
//! happens. The side of a module with settings is in `example.rs`; this file is the run.
//!
//! ```bash
//! export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres
//! cargo run --example tour
//! ```

mod example;

use std::time::Duration;

use example::{ExampleIam, IamSetting, days, deletion_window, shown};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;
use zgm_settings::{DefaultSetting, Setting, Settings, SettingsError, Value};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ---- Setting up: once, when the program starts ---------------------------------------

    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| "set DATABASE_URL to a PostgreSQL 18 database")?;
    let pool = PgPoolOptions::new().connect(&database_url).await?;
    // The module creates and updates its own table, in the schema `zgm_settings`.
    zgm_settings::migrate(&pool).await?;

    let settings = Settings::builder().pool(pool.clone()).build();
    // A module with settings gives its defaults when it starts. The first start writes them
    // to the database; a later one finds them there and leaves them.
    let _iam = ExampleIam::start(&settings).await?;

    println!("The defaults in the database:");
    for (name, value) in settings.defaults().await? {
        println!("  {name} = {}", shown(&value));
    }

    // Two companies. In a program their ids come from `zgm-iam`; the settings module knows
    // no tenants, so any UUID works. New ones on every run, so that runs do not meet.
    let nowak: Uuid = sqlx::query_scalar("SELECT uuidv7()")
        .fetch_one(&pool)
        .await?;
    let kowalski: Uuid = sqlx::query_scalar("SELECT uuidv7()")
        .fetch_one(&pool)
        .await?;
    // Everything of one company hangs from here; it does not ask the database.
    let in_nowak = settings.tenant_id(nowak);
    let in_kowalski = settings.tenant_id(kowalski);

    // ---- A company with no changes has the defaults ---------------------------------------

    println!("\nThe settings of Nowak, before any change:");
    print_list(&in_nowak.list().await?);

    // ---- Nowak changes a setting ----------------------------------------------------------

    let changed = in_nowak
        .set(IamSetting::UserDeletionWindow, Value::Duration(days(90)))
        .await?;
    println!("\nNowak keeps users 90 days before deletion. Changed: {changed}");
    let changed = in_nowak
        .set(IamSetting::UserDeletionWindow, Value::Duration(days(90)))
        .await?;
    println!("The same again, as after a second click. Changed: {changed}");
    in_nowak
        .set(
            IamSetting::WelcomeText,
            Value::Text("Witamy w firmie \"Nowak\"".to_owned()),
        )
        .await?;

    // What a module, e.g. IAM deleting a user, reads.
    let window = in_nowak.get(IamSetting::UserDeletionWindow).await?;
    println!("IAM reads for Nowak: {}", shown(&window));
    let window = in_kowalski.get(IamSetting::UserDeletionWindow).await?;
    println!(
        "IAM reads for Kowalski, who changed nothing: {}",
        shown(&window)
    );

    println!("\nThe settings of Nowak, as a screen shows them:");
    print_list(&in_nowak.list().await?);

    // ---- A person changes a default for every company -------------------------------------

    // E.g. on a screen of the program: 14 days instead of 30, at least a week, at most 60
    // days. The module refuses numbers that do not fit together, and writes nothing then.
    let shorter = DefaultSetting::Duration {
        value: days(14),
        min: Some(days(7)),
        max: Some(days(60)),
    };
    let changed = settings
        .change_default(IamSetting::UserDeletionWindow, shorter.clone())
        .await?;
    println!(
        "\nThe default of the deletion window is now 14 days, from 7 to 60. Changed: {changed}"
    );
    let changed = settings
        .change_default(IamSetting::UserDeletionWindow, shorter)
        .await?;
    println!("The same again. Changed: {changed}");
    let window = in_kowalski.get(IamSetting::UserDeletionWindow).await?;
    println!("IAM reads for Kowalski, at once: {}", shown(&window));
    // The bounds do not reach back: Nowak keeps 90 days, above the new most, until Nowak
    // sets another value.
    let window = in_nowak.get(IamSetting::UserDeletionWindow).await?;
    println!(
        "IAM reads for Nowak, who set 90 days before: {}",
        shown(&window)
    );
    refused(
        in_nowak
            .set(IamSetting::UserDeletionWindow, Value::Duration(days(90)))
            .await,
        "Nowak saving 90 days again",
    )?;

    // ---- Back to the default ----------------------------------------------------------------

    let restored = in_nowak
        .restore_default(IamSetting::UserDeletionWindow)
        .await?;
    println!("\nNowak restores the default of the deletion window. There was a row: {restored}");
    let restored = in_nowak
        .restore_default(IamSetting::UserDeletionWindow)
        .await?;
    println!("Again. There was a row: {restored}");
    let window = in_nowak.get(IamSetting::UserDeletionWindow).await?;
    println!("IAM reads for Nowak: {}", shown(&window));

    // ---- What the module refuses --------------------------------------------------------------

    println!("\nWhat the module refuses, writing nothing:");
    refused(
        in_nowak.get("iam.no_such_setting").await,
        "a name no module gave a default for",
    )?;
    refused(
        in_nowak
            .set(IamSetting::MaxUsers, Value::Text("ten".to_owned()))
            .await,
        "a text for a number",
    )?;
    refused(
        in_nowak
            .set(
                IamSetting::UserDeletionWindow,
                Value::Duration(Duration::from_millis(1500)),
            )
            .await,
        "a fraction of a second",
    )?;
    refused(
        in_nowak.set(IamSetting::MaxUsers, Value::Number(0)).await,
        "no users at all, below the least",
    )?;
    refused(
        settings
            .change_default(
                IamSetting::UserDeletionWindow,
                DefaultSetting::Duration {
                    value: days(30),
                    min: Some(days(60)),
                    max: Some(days(59)),
                },
            )
            .await,
        "a default whose least is above its most",
    )?;
    refused(
        settings
            .change_default(
                IamSetting::UserDeletionWindow,
                DefaultSetting::Duration {
                    value: days(400),
                    min: Some(days(1)),
                    max: Some(days(365)),
                },
            )
            .await,
        "a default above its own most",
    )?;
    refused(
        settings
            .change_default(
                IamSetting::UserDeletionWindow,
                DefaultSetting::Text("30 days".to_owned()),
            )
            .await,
        "a text for the default of a duration",
    )?;

    // ---- Leaving nothing behind -------------------------------------------------------------

    in_nowak.restore_default(IamSetting::WelcomeText).await?;
    // A later start of the module would leave the changed default, so the tour puts the
    // numbers of the module back itself.
    settings
        .change_default(IamSetting::UserDeletionWindow, deletion_window())
        .await?;
    let left = in_nowak.list().await?;
    println!(
        "\nAfter restoring the welcome text too, Nowak has changed {} settings.",
        left.iter().filter(|setting| setting.overridden).count()
    );
    Ok(())
}

/// Prints the settings of a company the way a screen might: the value, whether the company
/// changed it, and the bounds.
fn print_list(all: &[Setting]) {
    for setting in all {
        let mut notes = Vec::new();
        if setting.overridden {
            notes.push(format!(
                "changed; the default is {}",
                shown(&setting.default)
            ));
        }
        match (&setting.min, &setting.max) {
            (Some(min), Some(max)) => notes.push(format!("from {} to {}", shown(min), shown(max))),
            (Some(min), None) => notes.push(format!("at least {}", shown(min))),
            (None, Some(max)) => notes.push(format!("at most {}", shown(max))),
            (None, None) => {}
        }
        let notes = if notes.is_empty() {
            String::new()
        } else {
            format!(" ({})", notes.join("; "))
        };
        println!("  {} = {}{notes}", setting.name, shown(&setting.value));
    }
}

/// Prints an error the module was expected to give, and stops the tour if it gave none.
fn refused<T: std::fmt::Debug>(
    answer: Result<T, SettingsError>,
    what: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match answer {
        Err(error) => {
            println!("  {what}: {error}");
            Ok(())
        }
        Ok(value) => Err(format!("{what} was accepted: {value:?}").into()),
    }
}
