//! A tour of `zgm-limits`: one run that uses everything the module offers and prints what
//! happens. The product side (the limits, the keyed hash, the audit, the sign-in) is in
//! `example.rs`; this file is the run.
//!
//! ```bash
//! export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres
//! cargo run --example tour
//! ```

mod example;

use example::{AppAudit, AppLimit, ExampleHash, SignIn, sign_in};
use std::time::Duration;

use sqlx::postgres::PgPoolOptions;
use strum::VariantArray;
use uuid::Uuid;
use zgm_limits::{Answer, Blocked, Count, JobError, Limits, LimitsError, Period, Rule};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // ---- Setting up: once, when the application starts ----------------------------------

    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| "set DATABASE_URL to a PostgreSQL 18 database")?;
    // The pool as the README recommends it. By default `sqlx` checks a connection before every
    // use, which is a second round trip next to every query of the module; without the check
    // a refused attempt costs one. Connections that lie unused are closed after two minutes
    // instead, before anything between the application and the database drops them.
    let pool = PgPoolOptions::new()
        .test_before_acquire(false)
        .idle_timeout(Duration::from_secs(120))
        .connect(&database_url)
        .await?;
    // The module creates and updates its own tables, in the schema `zgm_limits`.
    zgm_limits::migrate(&pool).await?;
    // The application's own audit table, where the events of changes are written.
    AppAudit::create_table(&pool).await?;

    let limits = Limits::builder()
        .pool(pool.clone())
        // The hash of the hidden parts of keys (e-mails, addresses).
        .keyed_hash(ExampleHash)
        // Where the module reports what it did.
        .audit(AppAudit)
        // Every limit of the product: the rule of each is read here, so a mistake in a rule
        // stops the application now and not at the first attempt.
        .limits(AppLimit::VARIANTS)
        // The cleanup of old counters runs in the background; this is where its errors come.
        .run_jobs_in_background(|error: JobError| {
            eprintln!("limits job {} failed: {}", error.job, error.error);
        })
        .build();

    // A new tenant on every run, so that every run starts with no counter.
    let tenant_id = Uuid::new_v4();

    // ---- 1. The limits and their default rules ------------------------------------------

    section("1. The limits and their default rules");
    for default in limits.get_defaults() {
        let changeable = match default.bounds {
            Some(bounds) => format!(
                "a tenant may set {} to {} per {:?} to {:?}",
                bounds.min_at_most.get(),
                bounds.max_at_most.get(),
                bounds.min_per.as_duration(),
                bounds.max_per.as_duration()
            ),
            None => "a tenant cannot change it".to_owned(),
        };
        println!(
            "  {}: {}; {changeable}",
            default.limit_name,
            in_words(default.rule)
        );
    }

    // ---- 2. A limit per account: wrong passwords ----------------------------------------

    section("2. Wrong passwords block the account");
    let anna = Uuid::new_v4();
    // Three wrong passwords block the account. The fourth and fifth attempts are refused
    // before the password is checked, also the fifth, whose password is right.
    for password in ["1234", "qwerty", "password", "letmein", "correct horse"] {
        println!("  {password:?}");
        let outcome = sign_in(&limits, tenant_id, anna, password).await?;
        println!("    -> {}", said_to_the_user(&outcome));
    }

    // ---- 3. The list of blocked keys, and unblocking ------------------------------------

    section("3. An administrator sees who is blocked, and unblocks");
    let blocked = limits.get_blocked().tenant_id(tenant_id).call().await?;
    for blocked in &blocked.items {
        println!("  {}", listed(blocked));
        println!("  unblocking it");
        // The event of the unblock is written in the same transaction as the unblock.
        let unblocked = limits.unblock(tenant_id, blocked.id).await?;
        println!("    -> unblocked: {unblocked}");
    }
    println!("  \"correct horse\"");
    let outcome = sign_in(&limits, tenant_id, anna, "correct horse").await?;
    println!("    -> {}", said_to_the_user(&outcome));

    // ---- 4. A success on the last allowed attempt ---------------------------------------

    section("4. The right password on the last allowed attempt");
    // The attempt is counted before the password is checked, so the third attempt fills the
    // window (`KeyBlocked`) and the reset after the success lifts that block at once
    // (`BlockLifted` by `Reset`). The pair is an ordinary sign-in, not an incident.
    let jan = Uuid::new_v4();
    for password in ["1234", "qwerty", "correct horse"] {
        println!("  {password:?}");
        let outcome = sign_in(&limits, tenant_id, jan, password).await?;
        println!("    -> {}", said_to_the_user(&outcome));
    }

    // ---- 5. A key with a hidden part ----------------------------------------------------

    section("5. A hidden part of the key: codes sent to an e-mail address");
    // Nobody is signed in yet, so there is no account to count by: the key is the address.
    // `hidden` puts it in the database only as a hash, and first makes it uniform, so that
    // capital letters and spaces around it do not make a new key.
    for address in ["anna@example.com", "Anna@Example.com ", "ANNA@EXAMPLE.COM"] {
        let codes = limits
            .attempts(AppLimit::SignInCode)
            .tenant_id(tenant_id)
            .hidden(address)
            .call()
            .await?;
        println!("  a code for {address:?}");
        println!("    -> {}", answered(codes.hit().await?));
    }
    let codes = limits
        .attempts(AppLimit::SignInCode)
        .tenant_id(tenant_id)
        .hidden("anna@example.com")
        .call()
        .await?;
    println!("  a fourth code for the same address");
    println!("    -> {}", answered(codes.hit().await?));
    // `hidden_exact` takes the text as it is, for values where every character counts, such
    // as the identifier of an API key. The same text is then another key.
    let exact = limits
        .attempts(AppLimit::SignInCode)
        .tenant_id(tenant_id)
        .hidden_exact("Anna@Example.com ")
        .call()
        .await?;
    println!("  the same address taken exactly as written: another key");
    println!("    -> {}", answered(exact.hit().await?));

    // ---- 6. A limit for the whole tenant ------------------------------------------------

    section("6. A limit for the whole tenant: exports");
    // No account and no hidden part: the attempts of everybody in the tenant count together.
    let exports = limits
        .attempts(AppLimit::Export)
        .tenant_id(tenant_id)
        .call()
        .await?;
    for number in 1..=3 {
        println!("  export {number}");
        println!("    -> {}", answered(exports.hit().await?));
    }

    // ---- 7. Distinct elements -----------------------------------------------------------

    section("7. Different accounts tried from one address");
    // At most three different accounts from one address. The address is the hidden part of
    // the key, and the account is the element. An account that is already in the window
    // passes again; only a new one needs room.
    let from_address = limits
        .attempts(AppLimit::AccountsPerAddress)
        .tenant_id(tenant_id)
        .hidden("203.0.113.7")
        .call()
        .await?;
    for account in ["anna", "jan", "anna", "ewa", "piotr", "jan"] {
        println!("  {account}");
        println!(
            "    -> {}",
            answered(from_address.hit_distinct(account).await?)
        );
    }

    // ---- 8. The list of blocked keys, narrowed ------------------------------------------

    section("8. Everybody blocked in the tenant, and only some of them");
    let everybody = limits.get_blocked().tenant_id(tenant_id).call().await?;
    for blocked in &everybody.items {
        println!("  {}", listed(blocked));
    }
    // A page holds up to 100 keys, the newest block first. `next_cursor` is `Some` when there
    // are more; give it to `.maybe_after(...)` for the next page.
    println!("  more pages: {}", everybody.next_cursor.is_some());

    let code_limit: &'static str = AppLimit::SignInCode.into();
    let of_one_limit = limits
        .get_blocked()
        .tenant_id(tenant_id)
        .limit_name(code_limit)
        .call()
        .await?;
    println!("  only `{code_limit}`: {}", of_one_limit.items.len());
    let whole_tenant = limits
        .get_blocked()
        .tenant_id(tenant_id)
        .whole_tenant(true)
        .call()
        .await?;
    println!(
        "  only keys of the whole tenant: {}",
        whole_tenant.items.len()
    );

    // ---- 9. The rules of a tenant -------------------------------------------------------

    section("9. A tenant changes a rule");
    let stricter = Rule::Window {
        at_most: Count::new(1),
        per: Period::minutes(5),
    };
    println!("  wrong_password: one per five minutes");
    let changed = limits
        .override_rule(AppLimit::WrongPassword)
        .tenant_id(tenant_id)
        .rule(stricter)
        .call()
        .await?;
    println!("    -> changed: {changed}");

    // Outside the bounds the code gives the limit, and for a limit with no bounds at all.
    let too_loose = Rule::Window {
        at_most: Count::new(1000),
        per: Period::minutes(5),
    };
    for (limit, rule) in [
        (AppLimit::WrongPassword, too_loose),
        (AppLimit::Export, stricter),
    ] {
        let name: &'static str = limit.into();
        println!("  {name}: {}", in_words(rule));
        let refused = limits
            .override_rule(limit)
            .tenant_id(tenant_id)
            .rule(rule)
            .call()
            .await;
        println!("    -> {}", refusal(refused.map(|_changed| ())));
    }

    println!("  the rules in force in this tenant:");
    for rule in limits.get_rules().tenant_id(tenant_id).call().await? {
        let whose = if rule.overridden {
            "its own"
        } else {
            "the default"
        };
        println!("    {}: {whose}, {}", rule.limit_name, in_words(rule.rule));
    }

    // A new rule is for windows opened from now on. This account has no open window, so its
    // next one opens under the new rule: the second wrong password is refused. A window that
    // was open at the change would have kept the rule it was opened with, to its end.
    let ewa = Uuid::new_v4();
    for password in ["1234", "qwerty"] {
        println!("  {password:?}");
        let outcome = sign_in(&limits, tenant_id, ewa, password).await?;
        println!("    -> {}", said_to_the_user(&outcome));
    }

    println!("  back to the default");
    let restored = limits
        .restore_default(AppLimit::WrongPassword)
        .tenant_id(tenant_id)
        .call()
        .await?;
    println!("    -> restored: {restored}");

    // ---- 10. What the module refuses ----------------------------------------------------

    section("10. Mistakes the module answers with an error");
    // An error is never a permission: on any error the application refuses the action.
    let distinct = limits
        .attempts(AppLimit::AccountsPerAddress)
        .tenant_id(tenant_id)
        .hidden("203.0.113.7")
        .call()
        .await?;
    println!("  `hit` on a limit that counts distinct elements");
    println!("    -> {}", refusal(distinct.hit().await.map(|_answer| ())));

    let searches = limits
        .attempts(AppLimit::Search)
        .tenant_id(tenant_id)
        .call()
        .await?;
    println!("  a limit with a bucket");
    println!("    -> {}", refusal(searches.hit().await.map(|_answer| ())));

    println!("  the nil UUID as an account");
    let nobody = limits
        .attempts(AppLimit::WrongPassword)
        .tenant_id(tenant_id)
        .subject_id(Uuid::nil())
        .call()
        .await;
    println!("    -> {}", refusal(nobody.map(|_attempts| ())));

    // ---- 11. What the audit table holds -------------------------------------------------

    section("11. The application's audit table after this run");
    // Only changes are here: they were written in the transactions that made them. The
    // refusals and the blocks came after the fact and were only printed.
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT event FROM app_audit WHERE tenant_id = $1 ORDER BY id")
            .bind(tenant_id)
            .fetch_all(&pool)
            .await?;
    for (event,) in rows {
        println!("  {event}");
    }

    Ok(())
}

/// A heading of the tour.
fn section(title: &str) {
    println!("\n{title}");
}

/// A rule in words.
fn in_words(rule: Rule) -> String {
    match rule {
        Rule::Window { at_most, per } => {
            format!(
                "at most {} attempts per {:?}",
                at_most.get(),
                per.as_duration()
            )
        }
        Rule::WindowOfDistinct { at_most, per } => format!(
            "at most {} different elements per {:?}",
            at_most.get(),
            per.as_duration()
        ),
        Rule::Bucket {
            capacity,
            refill_every,
        } => format!(
            "a bucket of {} tokens, one back every {:?} (not counted yet)",
            capacity.get(),
            refill_every.as_duration()
        ),
    }
}

/// An answer of the module in words.
fn answered(answer: Answer) -> String {
    match answer {
        Answer::Allowed => "allowed".to_owned(),
        Answer::Refused { until } => format!("refused until {}", until.format("%H:%M:%S")),
    }
}

/// An error of the module, or the word that there was none.
fn refusal(result: Result<(), LimitsError>) -> String {
    match result {
        Ok(()) => "no error".to_owned(),
        Err(error) => format!("error: {error}"),
    }
}

/// A row of the list of blocked keys in words. A hidden part is never shown: the list has
/// only its hash.
fn listed(blocked: &Blocked) -> String {
    let whose = if blocked.whole_tenant {
        "the whole tenant".to_owned()
    } else if blocked.key_hash.is_empty() {
        format!("account {}", blocked.subject_id)
    } else {
        "a key with a hidden part".to_owned()
    };
    format!(
        "`{}` of {whose}, blocked until {}",
        blocked.limit_name,
        blocked.blocked_until.format("%H:%M:%S")
    )
}

/// What the product would tell the person who signs in.
fn said_to_the_user(outcome: &SignIn) -> String {
    match outcome {
        SignIn::SignedIn => "signed in".to_owned(),
        SignIn::WrongPassword => "wrong password".to_owned(),
        SignIn::TooManyAttempts { until } => {
            format!(
                "too many attempts, try again after {}",
                until.format("%H:%M:%S")
            )
        }
    }
}
