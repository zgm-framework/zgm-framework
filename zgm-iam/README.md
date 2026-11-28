# zgm-iam

The tenants of an application and their users, on PostgreSQL, built so that a copy of the database does not say who they are. A tenant is a UUID with its name encrypted by the application; a user is a UUID under a tenant. Both can be blocked, unblocked and deleted after a window, during which the deletion can be cancelled, and a loop of the module deletes them once their date has come.

Part of [zgm-framework](../README.md), a collection of modules used by [leads.run](https://leads.run).

> **Status:** version 0.1.0, the first release. The API may still change before 1.0. Every design choice is written down in [docs/adr](docs/adr/README.md).

- **Privacy first:** the table of tenants holds the name only as the bytes the application's cipher made of it, and a user has no name, no e-mail, nothing but its UUID. What says who a UUID is lives in the application.
- **Everything of a tenant hangs from `iam.tenant_id(id)`,** and everything of one user from `.user_id(id)`, so a function cannot reach past its tenant.
- **Three states for tenants and users:** `active`, `suspended`, `pending_deletion`. One function sets the state, so blocking, unblocking and cancelling a deletion are the same call.
- **Deletion after a window:** 30 days for a user and 90 for a tenant unless the tenant set otherwise, from settings of tenants with bounds the module gives.
- **A loop of the module deletes** what is past its date every two minutes, in batches, as the cleanup of `zgm-limits` does.
- **It creates its own tables,** and depends on no other module: the cipher and the settings come through traits.

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Guide](#guide)
  - [The cipher](#the-cipher)
  - [The settings of tenants](#the-settings-of-tenants)
  - [Tenants](#tenants)
  - [Users](#users)
  - [States and deleting](#states-and-deleting)
  - [The loop](#the-loop)
  - [Errors](#errors)
- [How it works](#how-it-works)
- [How it was tested](#how-it-was-tested)
- [Running the example and the tests](#running-the-example-and-the-tests)
- [Not there yet](#not-there-yet)

## Install

Requirements: PostgreSQL 18 (for `uuidv7()`), `tokio`, `sqlx` 0.9, Rust 1.94.

```toml
[dependencies]
zgm-iam = { git = "https://github.com/zgm-framework/zgm-framework", tag = "zgm-iam-v0.1.0" }

# For the settings of tenants, through the glue below:
zgm-settings = { git = "https://github.com/zgm-framework/zgm-framework", tag = "zgm-settings-v0.1.0" }
```

## Quick start

Everything below is taken from the example in [`examples/tour`](examples/tour), which runs every feature of the crate from start to end.

**1. Create the tables and build the module, once, when the program starts.**

```rust
use zgm_iam::{Iam, JobError, State};

zgm_settings::migrate(&pool).await?;
zgm_iam::migrate(&pool).await?;
let settings = zgm_settings::Settings::builder().pool(pool.clone()).build();

let iam = Iam::builder()
    .pool(pool.clone())
    .cipher(MyCipher)                      // encrypts the names of the tenants
    .settings(IamSettings(settings))       // the glue to zgm-settings, below
    .run_jobs_in_background(|error: JobError| eprintln!("{}: {}", error.job, error.error))
    .build()
    .await?;                               // writes the defaults of the settings, starts the loop
```

**2. Create a tenant and its users.**

```rust
let kowalski = iam.create_tenant("Kowalski sp. z o.o.").await?;
let in_kowalski = iam.tenant_id(kowalski.id);
let anna = in_kowalski.create_user().await?;
let page = in_kowalski.list_users().call().await?;        // the newest 100 users
```

**3. Block, delete and bring back.**

```rust
in_kowalski.user_id(anna.id).set_state(State::Suspended).await?; // Some(anna), blocked
in_kowalski.user_id(anna.id).delete().await?;                    // Some(anna), gone in 30 days
in_kowalski.user_id(anna.id).set_state(State::Active).await?;    // Some(anna), back
in_kowalski.delete().await?;                                     // Some(kowalski), gone in 90 days, users too
```

## Guide

### The cipher

The application implements `Cipher` with `encrypt` and `decrypt`, e.g. through its key service. The module encrypts the name of a tenant before it writes it, and decrypts it when it gives a tenant back from `set_state` or `delete`. The module has no key and no cipher of its own.

- **A name is required:** an empty one is `IamError::EmptyName`, and the cipher is not called. Its length and characters are the application's business, and two tenants may have the same name.
- **A cipher that gives empty bytes is broken** (`IamError::EmptyCiphertext`), and one that decrypts into bytes that are not UTF-8 gives `IamError::UnreadableName`. In both, nothing is written.

### The settings of tenants

The module reads how long a user and a tenant wait for deletion through the `TenantSettings` trait. It gives the defaults when it is built, with their bounds:

| Setting | Default | Least | Most |
|---|---|---|---|
| `IamSetting::UserDeletionWindow`, `iam.user_deletion_window` | 30 days | 1 day | 365 days |
| `IamSetting::TenantDeletionWindow`, `iam.tenant_deletion_window` | 90 days | 7 days | 365 days |

With `zgm-settings`, the glue is about 20 lines:

```rust
use zgm_iam::{DurationDefault, TenantSettings, TenantSettingsError};
use zgm_settings::{DefaultSetting, Value};

struct IamSettings(zgm_settings::Settings);

#[async_trait::async_trait]
impl TenantSettings for IamSettings {
    async fn add_default_duration(&self, name: &str, default: DurationDefault) -> Result<(), TenantSettingsError> {
        let DurationDefault { value, min, max } = default;
        let default = DefaultSetting::Duration { value, min, max };
        self.0.add_default(name, default).await.map(|_| ()).map_err(TenantSettingsError::new)
    }

    async fn duration(&self, tenant_id: Uuid, name: &str) -> Result<Duration, TenantSettingsError> {
        match self.0.tenant_id(tenant_id).get(name).await.map_err(TenantSettingsError::new)? {
            Value::Duration(duration) => Ok(duration),
            other => Err(TenantSettingsError::new(format!("{name} is not a duration: {other:?}"))),
        }
    }
}
```

- **`build()` waits for the defaults,** and fails with `IamError::Settings` when they cannot be written, e.g. when the database does not answer. `zgm-settings` writes a default the first time and leaves it there after that.
- **A tenant changes its own window in `zgm-settings`,** e.g. `settings.tenant_id(kowalski.id).set(IamSetting::UserDeletionWindow, Value::Duration(seven_days))`, and `zgm-settings` refuses a value outside the bounds. The names come from `IamSetting`, so a typo does not compile.
- **The module itself takes any length it gets;** only a date past the end of the year 9999 is `IamError::DeletionDateOutOfRange`.

### Tenants

| Function | What it does |
|---|---|
| `iam.create_tenant(name)` | writes a tenant, `active`, and gives it back with the id and the dates the database gave it |
| `iam.tenant_id(id)` | the functions of one tenant; it does not ask the database |
| `.set_state(state)` | blocks or unblocks the tenant, or brings it back from waiting for deletion; `None` when there is no such tenant |
| `.delete()` | the tenant waits for deletion, with all its users, for its window; `None` when there is no such tenant |

- **Nothing works in a tenant that is not active,** reading included: `create_user`, `list_users` and every function of a user give `IamError::TenantNotActive(state)`, and a tenant that does not exist `IamError::NoSuchTenant`. Only `set_state` and `delete` of the tenant itself work in any state.
- **The tenant comes back with its name decrypted** in the transaction of the change, so a cipher that fails changes nothing.

### Users

| Function | What it does |
|---|---|
| `in_tenant.create_user()` | writes a user, `active` |
| `in_tenant.list_users().call()` | a page of at most 100 users, the newest first; `.order(Order::OldestFirst)`, `.state(State::Suspended)` and `.maybe_after(page.next_cursor)` change it |
| `in_tenant.user_id(id)` | the functions of one user; it does not ask the database |
| `.get()` | the user, in any state, or `None` |
| `.set_state(state)` | blocks or unblocks the user, or brings it back from waiting for deletion |
| `.delete()` | the user waits for deletion for the window of its tenant |

- **A user of another tenant is `None`,** as is an id of no user, and nothing changes.
- **A user created while its tenant is being blocked** is written either before the block or not at all: the functions that change users lock the state of the tenant.

### States and deleting

- **`set_state(State::Active | State::Suspended)`** works from any state, also after the date of deletion, as long as the row is there. Leaving `pending_deletion` clears the date. The same state again changes nothing, not even `updated_at`.
- **`set_state(State::PendingDeletion)` is `IamError::NotSettable`:** only `delete` puts a user or a tenant there, as it gives the date.
- **`delete`** works on an active or a suspended user or tenant. The date is the moment of the change, by the clock of the database, plus the window. One that waits already keeps its date, whatever the window is now.
- **Who may set a state is the application's business.** Blocking, unblocking and cancelling are one function, so deleting and cancelling unblocks nobody the caller could not unblock anyway.

### The loop

`.run_jobs_in_background(on_error)` is a required step of the builder; `build()` starts the loop with `tokio::spawn`, so it must run inside a `tokio` runtime.

- **A pass at once, then one every 2 minutes.** It deletes, in every tenant whatever its state, the users in `pending_deletion` whose date has come, in batches of 100 with a pause of 300 ms after a full batch, and then the tenants past their date, one a statement, their users with them through the cascade.
- **Several copies of the application delete at once** without waiting for each other: the batches take rows with `FOR UPDATE SKIP LOCKED`.
- **An error goes to `on_error(JobError { job, error })`** and the loop goes on; a panic in `on_error` does not stop it either. `job` is `Job::DeleteUsersPastTheirDate` or `Job::DeleteTenantsPastTheirDate`.
- **The loop stops with the last clone of `Iam`.** A `TenantScope` or a `UserScope` holds a clone too.

### Errors

| Error | When |
|---|---|
| `NoSuchTenant` | a function of a tenant that does not exist, other than `set_state` and `delete`, which give `None` |
| `TenantNotActive(state)` | a function in a tenant that is suspended or waits for deletion |
| `NotSettable(state)` | `set_state(State::PendingDeletion)` |
| `EmptyName` | `create_tenant("")` |
| `Cipher(CipherError)`, `EmptyCiphertext`, `UnreadableName` | the application's cipher failed, gave empty bytes, or decrypted into bytes that are not UTF-8 |
| `Settings(TenantSettingsError)` | the application's settings failed, in `build()` or when a window is read |
| `DeletionDateOutOfRange` | the window puts the date past the end of the year 9999 |
| `UnknownState(word)` | the database holds a word that is no state; only SQL by hand writes one, and the row keeps it |
| `Database(sqlx::Error)`, `Migration(MigrateError)` | the database or `migrate` failed |

A function that returns an error writes nothing.

## How it works

**The tables.** `zgm_iam.tenants` has `id uuid` (UUID v7, the primary key), `name bytea`, `state text`, `deletes_at timestamptz` and the two dates. `zgm_iam.users` has `id`, `tenant_id` with a foreign key `ON DELETE CASCADE`, `state`, `deletes_at` and the dates. The database checks no state word: only the module writes the tables, and `State` holds the list. Indexes: the users of a tenant by id, the same by state, and two partial indexes of `deletes_at` for the rows in `pending_deletion`, which the loop goes along.

**Writing.** A function that changes a user locks the state of its tenant with `FOR SHARE` in the same transaction, so a block of the tenant at the same moment is waited for. A change of state is one `UPDATE … WHERE … AND state <> $state`, so the same state again writes nothing; when it changes nothing, a `SELECT` says why. A date of deletion is `statement_timestamp()` plus the window as an interval of microseconds.

**Deleting.** `DELETE … WHERE ctid = ANY(ARRAY(SELECT ctid … ORDER BY deletes_at LIMIT n FOR UPDATE SKIP LOCKED))`, as in `zgm-limits`: the `ORDER BY` keeps the database on the partial index under every plan. On 1,000,000 users, 20,000 of them past their date, a batch of 100 took 0.24 ms and a pass with nothing to delete 0.03 ms; a tenant of 500,000 users was deleted with them in 187 ms.

**Migrations.** `migrate` creates the schema `zgm_iam`, both tables, and the module's own history of migrations, apart from the program's. It is safe to call again and from several copies of a program at once.

## How it was tested

There are 102 tests in [`tests/`](tests), through the public API, on PostgreSQL 18, each on a new, empty database, among them both tables checked with SQL by hand and a block of a tenant at the same moment as the creation of a user.

The new tests of each step were run on the code before it and went red, except a few that checked what that code already did; the ADRs name them. In the steps up to `delete` the code was also broken on purpose, one change at a time, and a test went red for every change but one: an `ORDER BY` of the list that PostgreSQL does not need today, though SQL does not promise the order without it, so it stays. The later steps were checked by their tests alone. The ADRs list what was checked in each part.

## Running the example and the tests

The example and the tests need a PostgreSQL 18 database in `DATABASE_URL`. Commands are in the [`justfile`](justfile).

```bash
docker run -d --name iam-postgres -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:18
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres

just          # runs the tour: cargo run --example tour
just test     # the tests, each on a new, empty database
just build    # only compiles
```

The tour uses `zgm-settings` for the settings, in the same database, and a toy cipher that hides nothing. It prints what it does: the settings IAM gave, a tenant and three users, a user blocked, two deleted with the window of the tenant before and after the tenant shortened it, one brought back, the blocked users on a page, what the modules refuse, the tenant blocked and active again, and the tenant deleted. Its tenants are left waiting, and the loop deletes them after 90 days.

## Not there yet

- **Groups, roles, policies and the question "may this user do that",** as in AWS IAM.
- **Events for an audit,** as the `Audit` trait of `zgm-limits`.
- **Showing and listing tenants,** and changing the name of a tenant.
- **Ending the sessions of a user** deleted or blocked here: sessions belong to a module of their own.
- **Cleaning up** the rows of other modules, such as `zgm-settings`, when a tenant is deleted.

## Decisions

Every choice in this crate has a record in [docs/adr](docs/adr/README.md): the context, the options considered with their costs, the decision and why. The records describe the decisions in force today.

## Security

Please report vulnerabilities privately; see [SECURITY.md](../SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.
