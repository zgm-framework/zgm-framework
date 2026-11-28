# zgm-settings

The settings of an application's tenants, on PostgreSQL. Each module gives the defaults of its own settings, with the bounds a company may set them within, and a company changes only those it wants: how many days a user waits for deletion, how many users a company may have, whether people may sign up by themselves, the text a new user sees first.

The defaults and their bounds are in the database, one row per setting, written when a module first starts and changed later only through this module. What a company changed is a row of its own.

Part of [zgm-framework](../README.md), a collection of modules used by [leads.run](https://leads.run).

> **Status:** version 0.1.0, the first release. The API may still change before 1.0. Every design choice is written down in [docs/adr](docs/adr/README.md).

- **Defaults from the modules, kept in the database:** the first start of a module writes them, later starts leave them, and `change_default` changes them while the program runs.
- **Bounds:** a duration or a number may have a least and a most, and `set` refuses a value outside them. A value set before the bounds were narrowed stays as it was.
- **Names from the enum of a module,** with its prefix: `IamSetting::UserDeletionWindow` is `iam.user_deletion_window`.
- **Four kinds of value:** a duration, a whole number, yes or no, a text. A value of the wrong kind is refused, and a screen knows to show 90 days rather than 7776000.
- **No cache:** every call reads the database, so a change is seen at once by every copy of the program.
- **Everything of a company hangs from `settings.tenant_id(id)`,** so the company cannot be forgotten or swapped with another UUID.
- **It creates its own tables** and needs no JSON crate.

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

**1. Create the tables and the module, once, when the program starts.**

```rust
use zgm_settings::{DefaultSetting, Settings, Value};

zgm_settings::migrate(&pool).await?;
let settings = Settings::builder().pool(pool.clone()).build();
```

**2. A module with settings names them in an enum and gives their defaults when it starts.**

```rust
#[derive(Clone, Copy, strum::AsRefStr)]
#[strum(serialize_all = "snake_case", prefix = "iam.")]
pub enum IamSetting {
    UserDeletionWindow, // "iam.user_deletion_window"
    MaxUsers,           // "iam.max_users"
}

// `days(n)` is a helper of the example: Duration::from_secs(n * 24 * 60 * 60).
let window = DefaultSetting::Duration { value: days(30), min: Some(days(1)), max: Some(days(365)) };
settings.add_default(IamSetting::UserDeletionWindow, window).await?; // true the first time
let max_users = DefaultSetting::Number { value: 10, min: Some(1), max: None };
settings.add_default(IamSetting::MaxUsers, max_users).await?;
```

**3. Read, change and restore the settings of a company.**

```rust
let in_nowak = settings.tenant_id(nowak_id);

in_nowak.get(IamSetting::UserDeletionWindow).await?;      // Value::Duration(30 days): the default
in_nowak.set(IamSetting::UserDeletionWindow, Value::Duration(days(90))).await?; // true: changed
in_nowak.set(IamSetting::UserDeletionWindow, Value::Duration(days(400))).await; // Err(AboveTheMost)
in_nowak.list().await?;                                   // every setting, with its default and bounds
in_nowak.restore_default(IamSetting::UserDeletionWindow).await?; // true: 30 days again
```

**4. Change a default for every company that has not set its own,** e.g. from a screen of the program.

```rust
let shorter = DefaultSetting::Duration { value: days(14), min: Some(days(7)), max: Some(days(60)) };
settings.change_default(IamSetting::UserDeletionWindow, shorter).await?; // true
```

## Guide

### Defaults

A default is a `DefaultSetting`: the value of every company that has not changed the setting, and for a duration or a number the least and the most a company may set. Both bounds must be written, `None` for no bound; a flag and a text have none.

| Function | What it does |
|---|---|
| `add_default(name, default)` | a module gives the default of a setting when it starts; the first time it is written, later it is left as the database has it; `true` when written now |
| `change_default(name, default)` | changes the default and its bounds; `true` when anything changed |
| `defaults()` | every default in the database, in the order of the names, also of settings this program did not give |

- **The name is the prefix of the module and the name of the setting,** e.g. `iam.user_deletion_window`. An enum that derives `strum::AsRefStr` with `serialize_all = "snake_case"` and the module's `prefix` gives it; plain text works too and is kept exactly as given.
- **The default fixes the kind of the setting.** Every value of that setting, read or written, must be of the same kind.
- **A later start leaves the default the database has,** also when a new version of the module has other numbers in its code. Two versions of a program running side by side during a deployment do not overwrite each other's.
- **`add_default` checks nothing:** the numbers in the code of a module are trusted, and its tests check them. A duration loses its fraction of a second.
- **`change_default` checks what a person types:** the same kind as in the database, no fraction of a second, a least not above the most (equal is allowed), and a default within its bounds. It writes nothing otherwise. The values companies set stay as they are, also outside new bounds; the next `set` of every company is held against them.
- **No function deletes a default.** A setting a module dropped from its code keeps its row and stays on every list.

### Values and their kinds

| `Value` | For | In the database |
|---|---|---|
| `Duration(std::time::Duration)` | how long something lasts | its whole seconds: `7776000` for 90 days |
| `Number(i64)` | a whole number | the number: `250` |
| `Flag(bool)` | yes or no | `true` or `false` |
| `Text(String)` | a text | a JSON string: `"Witamy"` |

A duration has whole seconds. One with a fraction of a second is refused by `set` and `change_default` rather than cut off, because the table would keep another value than the one given. A bound of a duration is kept in whole seconds as a `bigint`.

### The settings of a company

`settings.tenant_id(id)` gives the module within one company. It does not ask the database, and the module does not know which companies exist: a company it never heard of has the defaults. It is `Clone`, so a program with one company can make it once, when it starts.

| Function | What it does |
|---|---|
| `get(name)` | the value the company set, or the default |
| `set(name, value)` | writes the value of the company, or changes it; `true` when anything changed, `false` when the company had that value already, and then not even `updated_at` moves |
| `restore_default(name)` | deletes the row of the company, so it has the default again; `true` when there was a row |
| `list()` | every default in the database, in the order of the names, each a `Setting { name, value, default, overridden, min, max }` |

- **The bounds hold from the moment they are set:** `set` refuses a value below the least or above the most in force, also the value the company has already. `get` and `list` give a value set before the bounds were narrowed as it is.
- **A value equal to the default, set by a company, is a row of its own.** The company keeps 30 days when the default is later changed to 14.
- **`restore_default` does not read the row,** so it also removes a row of the wrong kind, which only SQL written by hand can write.
- **`list` has every default in the database,** also of a setting a module dropped.

### Errors

| Error | When |
|---|---|
| `UnknownSetting(name)` | the name has no default in the database |
| `WrongKind { name, found }` | `set` or `change_default` was given another kind than the default, or the row of a company holds one; `found` is the value given or the JSON of the row |
| `UnreadableDefault { name, kind, found }` | the row of a default holds a kind the module does not know, or a value not of its kind; only SQL by hand writes one |
| `BelowTheLeast { name, min }`, `AboveTheMost { name, max }` | a value for `set`, or a default for `change_default`, outside the bounds; the bound is a `Value` of the kind of the setting, for a screen to show |
| `LeastAboveTheMost { name, min, max }` | `change_default` was given a least above the most |
| `FractionOfASecond(name)` | `set` or `change_default` was given a duration with a fraction of a second |
| `Database(sqlx::Error)` | the database failed |
| `Migration(MigrateError)` | `migrate` failed |

`set`, `restore_default` and `change_default` write nothing when they return an error.

## How it works

**The tables.** `zgm_settings.default_settings` has one row per setting: `name` as the primary key, `kind` (`duration`, `number`, `flag` or `text`), `value jsonb`, `min` and `max` as `bigint` with `NULL` for no bound, `created_at` and `updated_at`. `zgm_settings.tenant_settings` has `tenant_id uuid`, `name text`, `value jsonb` and the dates, with the primary key `(tenant_id, name)`: the company first, so that one index finds one setting of a company and all of its settings. Its `name` is a foreign key to the default, `ON DELETE RESTRICT`, with an index of its own. There is no foreign key to the companies, which live in another module. The database checks no kind and no bound; only the module writes the tables.

**Reading.** `get` is one statement: the default by its key, joined with the row of the company. The database takes the JSON apart (`jsonb_typeof`, `#>> '{}'`), and the module reads it as the kind of the default. `list` is one statement too, over every default, ordered by the bytes of the names (`COLLATE "C"`), as Rust orders text.

**Writing.** `add_default` is one `INSERT … ON CONFLICT (name) DO NOTHING`. `set` reads the default and its bounds, then writes in one statement, `INSERT … ON CONFLICT (tenant_id, name) DO UPDATE … WHERE … IS DISTINCT FROM …`, so the same value writes nothing. `change_default` reads the kind, then writes in one `UPDATE … WHERE (value, min, max) IS DISTINCT FROM …`. A text becomes a JSON string in the database (`to_jsonb`), so the module escapes nothing itself. `restore_default` is one statement that deletes the row of the company and finds the default.

**Migrations.** `migrate` creates the schema `zgm_settings`, both tables, and the module's own history of migrations, `zgm_settings._sqlx_migrations`, apart from the program's. It is safe to call again and from several copies of a program at once.

## How it was tested

There are 141 tests in [`tests/`](tests), through the public API, on PostgreSQL 18, each on a new, empty database, among them both tables checked with SQL by hand. 15 of them are under load: many copies of a program, each with a pool of its own, starting, reading, setting, restoring and changing defaults at once, and reads, sets and lists while a default or its bounds change.

The new tests of each step were run on the code before it, and went red, except a few that checked what that code already did; the ADRs name them. Then each part of the module was broken on purpose, one change at a time: the bounds swapped, off by one or not looked at, the key or the foreign key changed, the query without the company, a JSON `null` not handled, seconds above a `bigint` wrapped round, the same value written again, a default given again written over, the checks of `change_default` dropped or put after the write, and more. Each ADR lists the changes to its part and what caught them. One change passes every test of one copy at a time, checking that a default is missing before writing it without `ON CONFLICT`, and the tests of copies starting at once catch it.

**Measured** on one laptop with PostgreSQL 18 in Docker, 1,000 companies: one `get` takes about 0.3 ms at the median, one `set` 0.5 ms, one `list` of 50 settings 0.4 ms. The ADRs have the details.

## Running the example and the tests

The example and the tests need a PostgreSQL 18 database in `DATABASE_URL`. Commands are in the [`justfile`](justfile).

```bash
docker run -d --name settings-postgres -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:18
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres

just          # runs the tour: cargo run --example tour
just test     # the tests, each on a new, empty database
just build    # only compiles
```

The tour prints what it does, step by step: the defaults in the database, the settings of a company with their bounds before any change, a change and the same change again, what a module reads for a company that changed a setting and for one that did not, the list as a screen shows it, a change of a default for every company, restoring a default, and the errors. At the end it puts the default it changed back.

## Not there yet

- **The trait** through which a module such as `zgm-iam` reaches this one in its builder. It comes with the first module that reads its settings.
- **Cleaning up** the rows of companies deleted in another module.
- **A record of who changed a default,** and who may: the program decides that.
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
