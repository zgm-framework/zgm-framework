//! A tour of `zgm-iam`: one run that uses everything the module offers and prints what
//! happens. What an application writes to use the module is in `example.rs`; this file is the
//! run. It uses `zgm-settings` for the settings of the tenants, in the same database.
//!
//! ```bash
//! export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres
//! cargo run --example tour
//! ```

mod example;

use example::{DAY, IamSettings, ToyCipher};
use sqlx::postgres::PgPoolOptions;
use zgm_iam::{Iam, IamSetting, JobError, State, User};
use zgm_settings::{Settings, Value};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ---- Setting up: once, when the program starts ---------------------------------------

    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| "set DATABASE_URL to a PostgreSQL 18 database")?;
    let pool = PgPoolOptions::new().connect(&database_url).await?;
    // Each module creates and updates its own tables, in its own schema.
    zgm_settings::migrate(&pool).await?;
    zgm_iam::migrate(&pool).await?;

    let settings = Settings::builder().pool(pool.clone()).build();
    // `build()` gives the settings the defaults of the module and starts the loop that
    // deletes the users and the tenants past their date, every two minutes.
    let iam = Iam::builder()
        .pool(pool.clone())
        .cipher(ToyCipher)
        .settings(IamSettings(settings.clone()))
        .run_jobs_in_background(|error: JobError| eprintln!("{}: {}", error.job, error.error))
        .build()
        .await?;

    println!("The settings IAM gave, with their bounds:");
    for setting in settings.tenant_id(uuid::Uuid::nil()).list().await? {
        let of_iam = [
            IamSetting::UserDeletionWindow.as_ref(),
            IamSetting::TenantDeletionWindow.as_ref(),
        ];
        if of_iam.contains(&setting.name.as_str()) {
            println!(
                "  {} = {} (from {} to {})",
                setting.name,
                days(&setting.default),
                setting.min.as_ref().map_or("-".to_owned(), days),
                setting.max.as_ref().map_or("-".to_owned(), days),
            );
        }
    }

    // ---- A tenant and its users ---------------------------------------------------------------

    let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await?;
    println!(
        "\nA new tenant: {} {:?}, {:?}",
        kowalski.id, kowalski.name, kowalski.state
    );
    // Everything of one tenant hangs from here; it does not ask the database.
    let in_kowalski = iam.tenant_id(kowalski.id);
    let anna = in_kowalski.create_user().await?;
    let jan = in_kowalski.create_user().await?;
    let ola = in_kowalski.create_user().await?;
    println!(
        "Three users: Anna {}, Jan {}, Ola {}",
        anna.id, jan.id, ola.id
    );

    // ---- Blocking, deleting, bringing back ----------------------------------------------------

    // Everything of one user hangs from `user_id`; it does not ask the database either.
    let anna = in_kowalski
        .user_id(anna.id)
        .set_state(State::Suspended)
        .await?
        .ok_or("Anna is gone")?;
    println!("\nAnna is blocked: {:?}", anna.state);

    let jan = in_kowalski
        .user_id(jan.id)
        .delete()
        .await?
        .ok_or("Jan is gone")?;
    println!("Jan waits for deletion: {}", waits(&jan));

    // The tenant shortens its own window in `zgm-settings`, within the bounds IAM gave.
    let in_settings = settings.tenant_id(kowalski.id);
    in_settings
        .set(IamSetting::UserDeletionWindow, Value::Duration(DAY * 7))
        .await?;
    let ola = in_kowalski
        .user_id(ola.id)
        .delete()
        .await?
        .ok_or("Ola is gone")?;
    println!(
        "Kowalski sets 7 days; Ola waits for deletion: {}",
        waits(&ola)
    );

    let jan = in_kowalski
        .user_id(jan.id)
        .set_state(State::Active)
        .await?
        .ok_or("Jan is gone")?;
    println!(
        "Jan is brought back: {:?}, deletes at {:?}",
        jan.state, jan.deletes_at
    );

    let page = in_kowalski
        .list_users()
        .state(State::Suspended)
        .call()
        .await?;
    println!(
        "The blocked users of Kowalski: {:?}",
        page.items.iter().map(|user| user.id).collect::<Vec<_>>()
    );

    // ---- What the modules refuse --------------------------------------------------------------

    println!("\nWhat the modules refuse, writing nothing:");
    refused(
        in_settings
            .set(IamSetting::UserDeletionWindow, Value::Duration(DAY * 400))
            .await,
        "a window of 400 days, above the most",
    )?;
    refused(
        in_kowalski
            .user_id(anna.id)
            .set_state(State::PendingDeletion)
            .await,
        "waiting for deletion without delete",
    )?;
    let nowak = iam.create_tenant("Nowak S.A.").await?;
    let found = iam.tenant_id(nowak.id).user_id(anna.id).get().await?;
    println!("  Anna through another tenant: {found:?}");

    // ---- The whole tenant ---------------------------------------------------------------------

    let blocked = in_kowalski
        .set_state(State::Suspended)
        .await?
        .ok_or("Kowalski is gone")?;
    println!("\nKowalski is blocked: {:?}", blocked.state);
    refused(
        in_kowalski.create_user().await,
        "a new user of a blocked tenant",
    )?;
    in_kowalski.set_state(State::Active).await?;
    println!(
        "Kowalski is active again, and gets a user: {}",
        in_kowalski.create_user().await?.id
    );

    let deleted = in_kowalski.delete().await?.ok_or("Kowalski is gone")?;
    println!(
        "Kowalski waits for deletion, with all its users, until {}",
        deleted
            .deletes_at
            .map_or("never".to_owned(), |date| date.to_string())
    );
    iam.tenant_id(nowak.id).delete().await?;
    // Both tenants of this run are left waiting: the loop deletes them, with their users, in
    // 90 days.
    Ok(())
}

/// A duration in days, the way a screen might show it.
fn days(value: &Value) -> String {
    match value {
        Value::Duration(duration) if *duration == DAY => "1 day".to_owned(),
        Value::Duration(duration) => format!("{} days", duration.as_secs() / DAY.as_secs()),
        other => format!("{other:?}"),
    }
}

/// How long a user waits for deletion, in days.
fn waits(user: &User) -> String {
    match user.deletes_at {
        Some(date) => {
            let seconds = date.duration_since(user.updated_at).as_secs();
            format!("{} days, until {date}", seconds / DAY.as_secs() as i64)
        }
        None => "not at all".to_owned(),
    }
}

/// Prints an error a module was expected to give, and stops the tour if it gave none.
fn refused<T: std::fmt::Debug, E: std::fmt::Display>(
    answer: Result<T, E>,
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
