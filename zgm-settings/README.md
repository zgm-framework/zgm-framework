# zgm-settings

The settings of an application's tenants, on PostgreSQL. Each module gives the defaults of its own settings, and a company changes only those it wants: how many days a user waits for deletion, how many users a company may have, whether people may sign up by themselves, the text a new user sees first.

The database holds only what a company changed, one row per company and setting. Everything else is the default, which lives in the code of the module and reaches every company that has not changed it.

Part of [zgm-framework](../README.md), a collection of modules used by [leads.run](https://leads.run).

> **Status:** version 0.1, before the first release. The API may still change. Every design choice is written down in [docs/adr](docs/adr/README.md).

- **Defaults from the modules,** in memory, never written to the database, so a new default reaches every company that kept the old one.
- **Names from the enum of a module,** with its prefix: `IamSetting::UserDeletionWindow` is `iam.user_deletion_window`.
- **Four kinds of value:** a duration, a whole number, yes or no, a text. A value of the wrong kind is refused, and a screen knows to show 90 days rather than 7776000.
- **One query per call,** by the primary key, with no cache: a change is seen at once by every copy of the program.
- **Everything of a company hangs from `settings.tenant_id(id)`,** so the company cannot be forgotten or swapped with another UUID.
- **It creates its own table** and needs no JSON crate.

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Guide](#guide)
  - [Defaults](#defaults)
  - [Values and their kinds](#values-and-their-kinds)
  - [The settings of a company](#the-settings-of-a-company)
  - [Errors](#errors)
- [How it works](#how-it-works)
- [How it was tested](#how-it-was-tested)
- [Running the example and the tests](#running-the-example-and-the-tests)
- [Not there yet](#not-there-yet)

## Install

Requirements: PostgreSQL 18, `tokio`, `sqlx` 0.9, Rust 1.94.

```toml
[dependencies]
zgm-settings = { git = "https://github.com/zgm-framework/zgm-framework", tag = "zgm-settings-v0.1.0" }

# For the names of settings from an enum (plain text works too):
strum = { version = "0.28", features = ["derive"] }
```

## Quick start

Everything below is taken from the example in [`examples/tour`](examples/tour), which runs every feature of the crate from start to end.

**1. Create the table and the module, once, when the program starts.**

```rust
use zgm_settings::{Settings, Value};

zgm_settings::migrate(&pool).await?;
let settings = Settings::builder().pool(pool.clone()).build();
```

**2. A module with settings names them in an enum and gives their defaults when it is built.**

```rust
#[derive(Clone, Copy, strum::AsRefStr)]
#[strum(serialize_all = "snake_case", prefix = "iam.")]
pub enum IamSetting {
    UserDeletionWindow, // "iam.user_deletion_window"
    MaxUsers,           // "iam.max_users"
}

// `days(n)` is a helper of the example: Duration::from_secs(n * 24 * 60 * 60).
settings.add_default(IamSetting::UserDeletionWindow, Value::Duration(days(30)));
settings.add_default(IamSetting::MaxUsers, Value::Number(10));
```

**3. Read, change and restore the settings of a company.**

```rust
let in_nowak = settings.tenant_id(nowak_id);

in_nowak.get(IamSetting::UserDeletionWindow).await?;      // Value::Duration(30 days): the default
in_nowak.set(IamSetting::UserDeletionWindow, Value::Duration(days(90))).await?; // true: changed
in_nowak.get(IamSetting::UserDeletionWindow).await?;      // Value::Duration(90 days)
in_nowak.list().await?;                                   // every setting, with its default
in_nowak.restore_default(IamSetting::UserDeletionWindow).await?; // true: 30 days again
```

## Guide

### Defaults

`add_default(name, value)` gives the default of a setting: the value of every company that has not changed it. A module calls it when it is built, so a program that wants another default for every company says so in that module's builder.

- **The name is the prefix of the module and the name of the setting,** e.g. `iam.user_deletion_window`. An enum that derives `strum::AsRefStr` with `serialize_all = "snake_case"` and the module's `prefix` gives it; plain text works too and is kept exactly as given.
- **The default fixes the kind of the setting.** Every value of that setting, read or written, must be of the same kind.
- **A default given again under the same name replaces the one before.**
- **`defaults()`** lists all the defaults given so far, in the order of their names, without asking the database.
- **The clones of `Settings` share the defaults.** The program gives a clone to each module, and what a module gives through its clone, the program sees through its own.
- **Nothing of the defaults is written to the database,** so two versions of a program running side by side during a deployment do not overwrite each other's, and a new default reaches every company without a row at once.

### Values and their kinds

| `Value` | For | In the database |
|---|---|---|
| `Duration(std::time::Duration)` | how long something lasts | its whole seconds: `7776000` for 90 days |
| `Number(i64)` | a whole number | the number: `250` |
| `Flag(bool)` | yes or no | `true` or `false` |
| `Text(String)` | a text | a JSON string: `"Witamy"` |

A duration has whole seconds. One with a fraction of a second is refused rather than cut off, because the table would keep another value than the one given.

### The settings of a company

`settings.tenant_id(id)` gives the module within one company. It does not ask the database, and the module does not know which companies exist: a company it never heard of has the defaults. It is `Clone`, so a program with one company can make it once, when it starts.

| Function | What it does |
|---|---|
| `get(name)` | the value the company set, or the default |
| `set(name, value)` | writes the value of the company, or changes it; `true` when anything changed, `false` when the company had that value already, and then not even `updated_at` moves |
| `restore_default(name)` | deletes the row of the company, so it has the default again; `true` when there was a row |
| `list()` | every setting with a default, in the order of the names, each a `Setting { name, value, default, overridden }` |

- **A value equal to the default, set by a company, is a row of its own.** The company keeps 30 days when the program later changes the default to 14.
- **`set` checks only the kind,** not a range. Whether 0 days make sense is for the program, or the module that reads the setting, to check.
- **`restore_default` does not read the row,** so it also removes a row of the wrong kind, which only SQL written by hand can write.
- **`list` leaves out the rows of names no module gives a default for,** e.g. of a setting a module dropped.

### Errors

| Error | When |
|---|---|
| `UnknownSetting(name)` | no module gave a default for the name; the database is not asked |
| `WrongKind { name, found }` | `set` was given a value of another kind than the default, or the row holds one; `found` is the value given or the JSON of the row |
| `FractionOfASecond(name)` | `set` was given a duration with a fraction of a second |
| `Database(sqlx::Error)` | the database failed |
| `Migration(MigrateError)` | `migrate` failed |

`set` and `restore_default` write nothing when they return an error.

## How it works

**The table.** `zgm_settings.tenant_settings` has `tenant_id uuid`, `name text`, `value jsonb`, `created_at` and `updated_at`, with the primary key `(tenant_id, name)`: the company first, so that one index finds one setting of a company and all of its settings. There is no foreign key to the companies, which live in another module, so the rows of a company deleted there stay here. The database checks no name and no value beyond JSON; only the module writes the table.

**Reading.** `get` looks the default up in memory first, since without it the kind is unknown. Then one query by the primary key; the database takes the JSON apart (`jsonb_typeof`, `#>> '{}'`), and the module reads it as the kind of the default.

**Writing.** `set` is one statement, `INSERT … ON CONFLICT (tenant_id, name) DO UPDATE … WHERE … IS DISTINCT FROM …`, so the same value writes nothing. A text becomes a JSON string in the database (`to_jsonb`), so the module escapes nothing itself. `restore_default` is one `DELETE` by the primary key.

**Migrations.** `migrate` creates the schema `zgm_settings`, the table, and the module's own history of migrations, `zgm_settings._sqlx_migrations`, apart from the program's. It is safe to call again and from several copies of a program at once.

## How it was tested

There are 47 tests in [`tests/`](tests), through the public API: 7 on the defaults, without a database, and 40 on PostgreSQL 18, each on a new, empty database, among them the table itself checked with SQL by hand.

Every test was seen to fail first, before the code it tests existed. Then each part of the module was broken on purpose, one change at a time, 69 times in all: the key of the table in another order, the value as `text`, the dates from `now()`, the history of migrations in the default table, a clone that copies the defaults, the query without the company, a JSON `null` not handled, negative seconds wrapped round, the same value written again, the row written under another company, the names in reverse order, and more. Every time a test went red, except for one change that changes nothing (dropping `NOT NULL` from a column of the primary key). Two of the changes first passed, and the tests that catch them were added: a name in plain text with capital letters, and digits in a JSON string taken for a duration. The ADRs list the changes of each part.

**Not measured:** the time of a call. Each is one query through the primary key.

## Running the example and the tests

The example and the tests need a PostgreSQL 18 database in `DATABASE_URL`. Commands are in the [`justfile`](justfile).

```bash
docker run -d --name settings-postgres -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:18
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres

just          # runs the tour: cargo run --example tour
just test     # the tests, each on a new, empty database
just build    # only compiles
```

The tour prints what it does, step by step: the defaults a module gave, the settings of a company before any change, a change and the same change again, what a module reads for a company that changed a setting and for one that did not, the list as a screen shows it, restoring a default, and the errors.

## Not there yet

- **Ranges,** such as "from 1 to 365 days". If they come, the module of a setting gives them with its default and this module only enforces them; until then a program checks a value before `set`.
- **The trait** through which a module such as `zgm-iam` reaches this one in its builder. It comes with the first module that reads its settings.
- **Cleaning up** the rows of companies deleted in another module, and the rows of settings no module gives any more.
- **More kinds of value,** e.g. several numbers at once, or a fraction.
- **A separate database account for migrations,** and **a check at startup** that the migrations are applied.

## Decisions

Every choice in this crate has a record in [docs/adr](docs/adr/README.md): the context, the options considered with their costs, the decision and why. The records describe the decisions in force today.

## Security

Please report vulnerabilities privately; see [SECURITY.md](../SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.
