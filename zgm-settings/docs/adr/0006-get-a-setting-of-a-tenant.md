# ADR-0006: The functions of one tenant hang from `Settings::tenant_id`; `get` reads the default and the tenant's value from the database every time, in one statement, and gives the tenant's value as it is, also outside bounds lowered after it was set

> **Status:** accepted
> **Date:** 2026-11-26
> **Applies to:** `zgm-settings` (`src/tenant_scope.rs`, `src/repository.rs`, `Value::read_like`, `Value::read_as`, `SettingsError`, `tests/get.rs`, `tests/get_at_once.rs`)

## Context

- **A module reads a setting of a tenant,** e.g. `zgm-iam` the days a user of the tenant Nowak waits for deletion: 90 if Nowak has a row, else the default of 30.
- **The default lies in `zgm_settings.default_settings`, with its kind and its bounds, and the value of a tenant in `zgm_settings.tenant_settings`** ([ADR-0003](0003-the-tenant-settings-table.md)). A default may be changed in the database while the program runs, through `change_default` ([ADR-0002](0002-defaults-built-into-the-modules.md), [ADR-0010](0010-change-a-default.md)).
- **A program may run in several copies at once.**
- **In `zgm-iam` the functions of one tenant hang from `iam.tenant_id(id)`,** so that the tenant cannot be forgotten or swapped with another UUID. The maintainer asked for that there.
- **`zgm-limits` refuses a limit that is not on its list** (`UnknownLimit`), and `zgm-iam` refuses a word of state it does not know (`UnknownState`) rather than taking it for another state.

## Options considered

The first design was presented whole, with these choices, each following a pattern of the repository:

| Question | Chosen | Other option, and why not |
|---|---|---|
| Where the functions of one tenant hang | `settings.tenant_id(id).get(...)`, as in `zgm-iam` | `get(tenant_id, name)`: the tenant and other UUIDs can be swapped |
| How a value lies in `jsonb` | plain JSON: a duration in whole seconds (`7776000`), a number, `true` or `false`, a string | tagged, e.g. `{"duration": 7776000}`: longer, and the kind is in the row of the default anyway |
| A name without a default | the error `UnknownSetting` | `Option<Value>`: every caller would handle a `None` that only a mistake in the program can cause |
| A row with a value of another kind than the default | the error `WrongKind` | the default: a wrong row, which only SQL by hand can write, would pass unnoticed |
| A tenant the module never heard of | the defaults | an error: the module does not know the tenants, which live in `zgm-iam` |
| How the JSON is read | the database takes it apart: `jsonb_typeof(value)` and `value #>> '{}'` | `serde_json`: one more crate for four kinds of scalar |

**Where `get` takes the default from:**

| Option | Pros | Cons |
|---|---|---|
| **A. From the database, every time, in the statement that reads the row of the tenant** | a default changed in the database is seen at once, by every copy of the program; still one statement | the statement reads two rows instead of one |
| **B. From memory, as `add_default` kept it** | the statement reads one row | a change of a default reaches a copy of the program only when it starts again |

**The value of a tenant above a most lowered after it was set,** e.g. Nowak set 90 days while the most was 365, and the most was then lowered to 60:

| Option | Pros | Cons |
|---|---|---|
| **A. The value as it is, 90** | the tenant gets what it set; no code to cut values | a tenant may keep a value the bounds no longer allow, until it sets another |
| **B. The value cut to the bound, 60, while the row keeps 90** | the new bound holds for every tenant at once | code to cut values, and a list of settings would show two numbers, the one set and the one in force |

## Decision

**As in the first table, where: A, and the value: A** (the first design accepted by the maintainer on 2026-11-23: "ok"; on 2026-11-26 on where: "no, it is to take it from the database every time, without a cache, right?"; and on the value: "it gets A; when somebody changes the max, it guards only new changes and does not check old ones backwards, the way a law does not work backwards").

```rust
let in_nowak = settings.tenant_id(nowak.id);
let window = in_nowak.get(IamSetting::UserDeletionWindow).await?;
// Value::Duration(90 days) with Nowak's row, the default 30 days without it
```

- **`Settings::tenant_id(id)` gives a `TenantScope`** without asking the database. It holds its own copy of `Settings` and is `Clone`.
- **`get(name)` is one statement:** the row of the default by its key, and the row of the tenant by its key, joined to it, `default_settings d LEFT JOIN tenant_settings t ON t.tenant_id = $1 AND t.name = d.name WHERE d.name = $2`. Nothing is kept between calls.
- **No row of the default is `SettingsError::UnknownSetting(name)`.** A row of a tenant cannot exist without one ([ADR-0003](0003-the-tenant-settings-table.md)).
- **The default is read as the kind its row names** (`Value::read_as`), and a row the module cannot read, of an unknown kind or with a value not of its kind, is `SettingsError::UnreadableDefault`, with or without a row of the tenant.
- **A row of the tenant is read as the kind of the default** (`Value::read_like`): a duration from a JSON number that parses as `u64` seconds, a number from a JSON number that parses as `i64`, a flag from a JSON boolean, a text from a JSON string without its quotes and escapes. Anything else is `SettingsError::WrongKind { name, found }`, with the JSON as the database writes it.
- **The bounds are not looked at.** The value of a tenant is given as it is.
- **`SettingsError::Database`** is a failure of the database.
- **The tests use `uuid` 1.26.1,** and the module takes it for `tenant_id`, with the `uuid` feature of `sqlx`.

## Why

- **From the database every time:** the maintainer asked for it, and it is what makes a change of a default, made while the program runs, reach every copy at once. Reading both rows in one statement keeps `get` at one round trip.
- **The value as it is:** a bound guards what a tenant sets from the moment it holds, and does not reach back to what was set before it, in the maintainer's words "the way a law does not work backwards".
- **The kind comes from the row of the default,** so the row of a tenant needs no tag, and the value stays as readable as in the examples the maintainer accepted: `7776000` for 90 days.
- **Taking the JSON apart in the query** keeps the module without a JSON crate: `#>> '{}'` gives a number or a boolean as written and a string without its quotes, also with quotes inside (`"Firma \"Nowak\""` gives `Firma "Nowak"`). It gives SQL `NULL` for a JSON `null`, so `coalesce` turns that into the text `null`, which is no kind the module knows; without it the read failed with an error of the database. Without a row of the tenant its JSON is SQL `NULL`, which is how the module tells the two apart.
- **`uuid` 1.26.1 is the version `zgm-limits` uses.** Checked on 2026-11-23: not yanked, OSV knows no vulnerabilities in it, the repository is not archived; 1.29.3 exists, and the repository keeps one version in all its modules.
- **Checked by tests** in `tests/get.rs`, on a database: without a row a tenant gets the default of every kind; with a row it gets its value of every kind, among them 0 seconds, `i64::MIN` and a text with quotes inside; the row of another tenant changes nothing; a change of the row is seen by the next read, and a deleted row gives the default again; a name without a default in the database is `UnknownSetting`; a setting this program did not give, whose default an earlier version left, is read from the database; a copy of the program that gave no default reads them; a change of the default in the database is seen by the next read; a value above a most lowered after it was set stays the value of the tenant; a default of an unknown kind is `UnreadableDefault` with a row of the tenant and without; a value of another kind is `WrongKind` with the name and the JSON (a text, digits in a string, a negative number, a fraction and `true` for a duration; a fraction, a number beyond `i64` and a number in a string for a number; `1` and `"true"` for a flag; a number, an object and `null` for a text).
- **Checked under load** in `tests/get_at_once.rs`: 100 tenants read at once, 10 times each, and each gets its own value or the default. 16 tenants read 200 times each while the default changes once a quarter of the reads are done: every read gives the old or the new value, once a reader has the new one it never has the old one again, and afterwards all give the new one.
- **Seen to fail first:** 4 of the new tests went red on the `get` that took the default from memory (the setting this program did not give, the copy that gave no default, the change of the default, the unreadable default). The other 2 new ones passed at once: a name without a default was refused before too, and the old `get` did not look at bounds either; they stay to hold the decision. Then `get` was broken on purpose, one change at a time, 11 times (an inner join, the row of any tenant, the row of any setting of the tenant, the first default by name, the value of the tenant read from the default, the row of the tenant never found, a missing row taken for one, the default from memory, the text instead of the JSON in the error, the value of the tenant ignored, an unreadable default taken for an unknown setting), and every time a test went red. The first run of these hung: with the inner join every read failed, and the test of reads during a change waited for reads that had stopped. It now counts a failed read too, and fails within a second.
- **Measured** on one laptop with PostgreSQL 18 in Docker, 1,000 tenants of which half have a row, before and after the change: one `get` at a time took 0.29–0.30 ms at the median before and 0.30–0.31 ms after; 16 at once gave 9,500–10,700 calls a second before and 9,700–10,100 after. The difference is within what runs differ by.

## Consequences

- **Every `get` is one statement that reads two rows by their keys.** Nothing is cached, so a change of a row of a tenant or of a default is seen at once by every copy of the program.
- **A tenant may keep a value outside the bounds in force,** when the bounds changed after it set it, until it sets another.
- **A tenant that does not exist has the defaults.** The module cannot tell it from a tenant that changed nothing.
- **A duration has whole seconds.** A fraction of a second cannot be stored or read.
- **A setting a module dropped can still be read,** as long as its default stays in the database; only the code that knows its name would ask for it.
