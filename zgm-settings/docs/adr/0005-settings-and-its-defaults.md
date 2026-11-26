# ADR-0005: `Settings` is built with a `bon` builder and keeps nothing but the pool; a module gives each default with its bounds as a `DefaultSetting` to `add_default`, which writes the row once in one statement and checks nothing; `defaults` reads them from the database

> **Status:** accepted
> **Date:** 2026-11-26
> **Applies to:** `zgm-settings` (`src/settings.rs`, `src/default_setting.rs`, `src/value.rs`, `repository::insert_default`, `repository::select_defaults`, `tests/defaults.rs`, `tests/defaults_at_once.rs`)

## Context

- **The defaults and their bounds are built into the modules, written to the database at the first start and left there after that** ([ADR-0002](0002-defaults-built-into-the-modules.md)). A duration or a number has both bounds, each of them possibly none; a flag or a text has none.
- **A default lies in `zgm_settings.default_settings`,** with its kind, its value as `jsonb` and its bounds as `bigint` ([ADR-0003](0003-the-tenant-settings-table.md)), e.g. `duration`, `2592000`, `86400` and `31536000` for 30 days, at least a day and at most a year.
- **A setting is named with the prefix of its module,** e.g. `iam.user_deletion_window`, from an enum of the module through `strum` ([ADR-0003](0003-the-tenant-settings-table.md)).
- **Several copies of a program may start at once,** e.g. during a deployment, and give the same defaults, also with different numbers when their versions differ.
- **`zgm-limits` and `zgm-iam` are built with `bon` builders,** once when the program starts, and cloned where they are needed.

## Options considered

**What a value is in Rust:**

| Option | Pros | Cons |
|---|---|---|
| **A. Plain JSON (`serde_json::Value`), with "time is always in seconds" agreed in a record** | one type through the whole module; a module turns JSON into its own type in a few lines | nobody checks that the modules keep to the agreement; a screen of settings shows `7776000` and does not know it is time; one more crate |
| **B. The module's own type, which the settings module turns into JSON and back through `serde`** | convenient in the module that reads the setting | the list of all the settings of a tenant needs one type anyway, so the module would have two ways; `serde` in every type |
| **C. A few kinds the settings module knows: time, a number, yes or no, a text** | the agreement on time is in the code, not on paper; a screen can show 90 days, a box for yes or no, a field for a number; a value of another kind than the default's can be refused | a new kind, e.g. several numbers at once, is a change of the settings module |

**How a module gives a default with its bounds:**

| Option | Pros | Cons |
|---|---|---|
| **A. One function, `add_default(name, DefaultSetting)`, with the bounds in named fields** | a bound cannot be left out: without `max` it does not compile, so no bound is written `max: None`; the field says which bound is which | longer to write |
| **B. A function for each kind, the bounds as the arguments after the value** | shorter | the call does not say which number is the least; the two swapped compile too |

**What `add_default` checks** (a least above the most, a default outside its own bounds, a fraction of a second, which the table does not keep):

| Option | Pros | Cons |
|---|---|---|
| **A. All of it: an error, nothing written** | a mistake in a module shows at its first start | code and tests for mistakes that the module's own tests would catch anyway |
| **B. Nothing: written as given, the fraction cut off** | the least code | a mistake in a module reaches the database at the first start and stays there |

**What `Settings` keeps in memory after a default is given:**

| Option | Pros | Cons |
|---|---|---|
| **A. What the database has, read after writing** | a later start, also of a version with other numbers, gets the number of the database; `defaults()` asks no database | a second statement for every default when a module starts; a default changed in the database reaches a copy only when it starts again |
| **B. What was given** | one statement | a later start answers with the number of its code while the database has another |
| **C. Nothing: every function reads the database** | one place every default comes from, for every copy at once; one statement for every default | `defaults()` asks the database, so it is `async` and can fail |

## Decision

**Value: C. Giving: A. Checks: B. Memory: C** (decided by the maintainer: on the value on 2026-11-23, after "we would need to assume that things of this kind are always in seconds or something", "then 2"; on giving on 2026-11-26, "A"; on the checks, "no, no checks here any more, we trust that nobody makes a mistake here when implementing it", together with "and the same if they add a default with correct data once and it is already in the database, and then with the next release they change it and it is incorrect, then nothing happens either and the entry in the database is not updated, because it is already there, right?"; the whole design, with the memory of A, "yes"; and on 2026-11-26, when `get` came to read the database, "no, it is to take it from the database every time, without a cache, right?", and when `list` did, "well, delete it then, everything goes through the settings module now, right? go on", after which nothing read the memory any more).

```rust
let settings = Settings::builder().pool(pool).build();
let window = DefaultSetting::Duration { value: thirty_days, min: Some(a_day), max: None };
let written = settings.add_default(IamSetting::UserDeletionWindow, window).await?;
settings.defaults().await?; // {"iam.user_deletion_window": Duration(30 days)}
```

- **`Value` has four kinds:** `Duration(std::time::Duration)`, `Number(i64)`, `Flag(bool)` and `Text(String)`. How each lies in the `jsonb` column is in [ADR-0006](0006-get-a-setting-of-a-tenant.md).
- **`DefaultSetting` has the same four:** `Duration { value, min, max }` and `Number { value, min, max }` with `Option` bounds, `Flag(bool)` and `Text(String)` without.
- **`add_default(name, default)` is `async` and returns `Result<bool, SettingsError>`.** It writes the row with `INSERT … ON CONFLICT (name) DO NOTHING`, so the first default of a name is written and every later one leaves the row as it is; `true` means it was written now. The name is `impl AsRef<str>`, so a variant of an enum that derives `strum::AsRefStr` gives it, and plain text works too; it is kept exactly as given.
- **That is the one statement, and a row that is there is not read.** Two copies of a program giving a default at once write one row: the second waits for the first and writes nothing.
- **Nothing given is checked.** A duration is written in whole seconds, the fraction cut off; a bound of more seconds than a `bigint` holds is written as the most it holds, about 292 billion years, and a value of any number of seconds is kept, as JSON holds it.
- **An error writes nothing.** `SettingsError::Database` when the database fails, e.g. it does not answer, the migration has not run, or a text holds a NUL, which PostgreSQL does not keep.
- **`defaults()` is `async` and reads every default from the database,** as a `BTreeMap` by name, in the order of the names, also of settings this program did not give. A row the module cannot read, which only SQL by hand can write, is `SettingsError::UnreadableDefault`. `zgm-limits` has the like in `get_defaults()`.
- **`Settings` holds the pool of connections and nothing else,** which its builder takes. Every clone and every copy of the program sees the same defaults, as they come from the database every time.

## Why

- **The kinds:** the maintainer's point that time needs one agreed form is kept best by the code that writes it. The settings module then knows the kind of every setting, which a screen and a check of `set` both need.
- **`std::time::Duration`:** a plain Rust type, with no crate and no negative lengths. `zgm-iam` keeps its dates with `jiff`; turning a `jiff` length into this one is its own work.
- **`i64` for a number:** a whole number covers counts and limits; fractions come when a setting needs them.
- **Named fields:** both bounds must be written, and the name of each says which it is.
- **No checks:** the numbers come from the code of a module, whose author is trusted with them, and its own tests see them first.
- **Nothing in memory:** the default is the one the database has, now, for every copy of the program, and a new number in a module's code does not change it ([ADR-0002](0002-defaults-built-into-the-modules.md)). The maintainer asked for no cache.
- **`true` or `false`:** a program can tell in its log whether this start wrote a default, and the tests can tell which of two copies did.
- **`bon` 3.10.2 is the version `zgm-limits` uses.** Checked on 2026-11-23: not yanked, OSV knows no vulnerabilities in it, its repository is not archived; 3.10.3 exists, and the repository keeps one version in all its modules. The tests use `strum` 0.28.0, checked the same way.
- **Checked by tests** in `tests/defaults.rs`, on PostgreSQL 18 but for two: a database without defaults has none; a copy of the program that gave no default reads them all, also one no module gives now; the first default of each kind is written with its kind and its bounds, and `add_default` says `true`; a default given again leaves the row and its date, says `false`, and `defaults()` gives the first; the next version of a module, with another number, gets the default of the database, also through `get`; a default changed in the database is the one the module gets; a default of another kind given again leaves the row of the first kind; a missing bound is written as no bound; a least above the most and a value outside its bounds are written as given; a fraction of a second is cut off; a bound of more seconds than a `bigint` holds is written as the most it holds, and `Duration::MAX` as a value is kept; the least and the greatest `i64` are kept; a text is kept exactly (empty, with quotes, a backslash, two lines and a tab, Polish letters, an emoji, spaces round it, SQL, a text that looks like JSON, the word `null`); a text with a NUL is an error and nothing is written; a name is kept exactly (with capitals and a space, with Polish letters, empty); the defaults of several modules are all there in the order of their names; a clone sees what another gave; two copies of a program giving a default at once write it once; a database that does not answer and a database without the migration are errors for `add_default` and for `defaults()`; a row with an unknown kind, a text for a duration, negative or fractional seconds, a JSON `null`, or `true` for a number stays when the default is given again, and `defaults()` refuses it as `UnreadableDefault`.
- **Checked under load** in `tests/defaults_at_once.rs`: 16 copies of a program, each with a pool of its own, start at once and give the same 50 defaults with numbers of their own; each default is written once, by one copy, and every copy has that copy's number. 200 defaults given at once through clones of one `Settings` are all written. 20 starts one after another, each with other numbers, leave every row as the first wrote it.
- **Every new test was seen to fail first,** when `add_default` came to write the database: 16 of the 20 in `tests/defaults.rs` on a version that kept the defaults in memory only; the other 4 checked only the memory, which worked before. Then the code was broken on purpose, one change at a time, 26 times (among them the bounds swapped or dropped, a bound above a `bigint` as 0 or wrapped round, a flag with a bound, a duration written as a number, the words of the kinds changed, a row read as another kind, seconds written as milliseconds, a default given again written over, `true` always, the bounds swapped in the SQL, the read of any row, the value given kept in memory instead of the one of the database, the memory written before the database, an unreadable row taken as the value given, the kind or the JSON in the error taken from elsewhere), and every time a test went red. When the memory went, 4 tests that read the defaults went red on a `defaults()` that still read memory (a copy that gave none, a database that does not answer, one without the migration, an unreadable row); the changes to the memory and to the read after writing went with the code. Without the read after writing, `add_default` always saying `true` was broken again on purpose, and went red. One more change passes every test that gives a default from one copy at a time: checking that the row is missing and then writing it, without `ON CONFLICT`. Both tests of copies starting at once caught it, each three times out of three, as the database refused the second row.
- **Measured** on one laptop with PostgreSQL 18 in Docker, with the read after writing: one `add_default` took 0.42–0.45 ms at the median, written now or already there, and at most 5.3 ms in 1,000 calls; a start of a module with 4 defaults took 2.0 ms the first time and 1.4 ms later, with 50 defaults 22 ms and 18 ms; 16 copies starting at once with 50 defaults each were all done in 111–144 ms, a call taking 2.2–2.6 ms at the median and at most 10 ms.Without it, in one statement: one `add_default` takes 0.31–0.39 ms at the median; a start with 4 defaults 2.3 ms the first time and 1.3 ms later, with 50 defaults 24 ms and 19 ms, and 16 copies at once 101–119 ms, the starts within what runs differ by.

## Consequences

- **A mistake in the numbers of a module is written at its first start and stays.** Only a function of the module changes the row after that.
- **Starting a module is one statement for each of its defaults,** later starts too.
- **A text with a NUL cannot be a default.** PostgreSQL refuses it, and `add_default` returns the error.
- **A row the module cannot read shows when a setting is read,** not when its module starts: `add_default` does not read the row that is there.
- **A setting of several numbers at once** needs a new kind of `Value` first.
- **Nothing checks that a name has a prefix:** any text is a name, the empty one too.
