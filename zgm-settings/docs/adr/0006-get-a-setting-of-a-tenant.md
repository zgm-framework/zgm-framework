# ADR-0006: The functions of one tenant hang from `Settings::tenant_id`; `get` gives the tenant's value or the default, read as the kind of the default from plain JSON

> **Status:** accepted
> **Date:** 2026-11-23
> **Applies to:** `zgm-settings` (`src/tenant_scope.rs`, `src/repository.rs`, `Value::read_like`, `SettingsError`, `tests/get.rs`)

## Context

- **A module reads a setting of a company,** e.g. `zgm-iam` the days a user of the company Nowak waits for deletion: 90 if Nowak has a row, else the default of 30.
- **The defaults are in memory, and each has its kind** ([ADR-0005](0005-settings-and-its-defaults.md)); the value of a company is `jsonb` in its row ([ADR-0003](0003-the-tenant-settings-table.md)).
- **In `zgm-iam` the functions of one tenant hang from `iam.tenant_id(id)`,** so that the tenant cannot be forgotten or swapped with another UUID. The maintainer asked for that there.
- **`zgm-limits` refuses a limit that is not on its list** (`UnknownLimit`), and `zgm-iam` refuses a word of state it does not know (`UnknownState`) rather than taking it for another state.

## Options considered

The design was presented whole, with these choices, each following a pattern of the repository:

| Question | Chosen | Other option, and why not |
|---|---|---|
| Where the functions of one tenant hang | `settings.tenant_id(id).get(...)`, as in `zgm-iam` | `get(tenant_id, name)`: the tenant and other UUIDs can be swapped |
| How a value lies in `jsonb` | plain JSON: a duration in whole seconds (`7776000`), a number, `true` or `false`, a string | tagged, e.g. `{"duration": 7776000}`: longer, and the kind is known from the default anyway |
| A name no module gave a default for | the error `UnknownSetting`, also when a row of that name exists | `Option<Value>`: every caller would handle a `None` that only a mistake in the program can cause |
| A row with a value of another kind than the default | the error `WrongKind` | the default: a wrong row, which only SQL by hand can write, would pass unnoticed |
| A tenant the module never heard of | the defaults | an error: the module does not know the tenants, which live in `zgm-iam` |
| How the JSON is read | the database takes it apart: `jsonb_typeof(value)` and `value #>> '{}'` | `serde_json`: one more crate for four kinds of scalar |

## Decision

**As above** (accepted by the maintainer on 2026-11-23 with the design: "ok").

```rust
let in_nowak = settings.tenant_id(nowak.id);
let window = in_nowak.get(IamSetting::UserDeletionWindow).await?;
// Value::Duration(90 days) with Nowak's row, the default 30 days without it
```

- **`Settings::tenant_id(id)` gives a `TenantScope`** without asking the database. It holds its own copy of `Settings` and is `Clone`.
- **`get(name)` looks the default up first.** Without one it is `SettingsError::UnknownSetting(name)`, and the database is not asked.
- **Then one query by the primary key:** `SELECT jsonb_typeof(value), coalesce(value #>> '{}', 'null'), value::text FROM zgm_settings.tenant_settings WHERE tenant_id = $1 AND name = $2`. No row gives the default.
- **A row is read as the kind of the default** (`Value::read_like`): a duration from a JSON number that parses as `u64` seconds, a number from a JSON number that parses as `i64`, a flag from a JSON boolean, a text from a JSON string without its quotes and escapes. Anything else is `SettingsError::WrongKind { name, found }`, with the JSON as the database writes it.
- **`SettingsError::Database`** is a failure of the database.
- **The tests use `uuid` 1.26.1,** and the module takes it for `tenant_id`, with the `uuid` feature of `sqlx`.

## Why

- **The kind comes from the default,** so the row needs no tag, and the value stays as readable as in the examples the maintainer accepted: `7776000` for 90 days.
- **The default is looked up before the database,** because without it the kind is unknown and the row cannot be read anyway.
- **Taking the JSON apart in the query** keeps the module without a JSON crate: `#>> '{}'` gives a number or a boolean as written and a string without its quotes, also with quotes inside (`"Firma \"Nowak\""` gives `Firma "Nowak"`). It gives SQL `NULL` for a JSON `null`, so `coalesce` turns that into the text `null`, which is no kind the module knows; without it the read failed with an error of the database.
- **`uuid` 1.26.1 is the version `zgm-limits` uses.** Checked on 2026-11-23: not yanked, OSV knows no vulnerabilities in it, the repository is not archived; 1.29.3 exists, and the repository keeps one version in all its modules.
- **Checked by tests,** on a database: without a row a company gets the default of every kind; with a row it gets its value of every kind, among them 0 seconds, `i64::MIN` and a text with quotes inside; the row of another company changes nothing; a change of the row is seen by the next read, and a deleted row gives the default again; a name without a default is `UnknownSetting` with a row and without; a value of another kind is `WrongKind` with the name and the JSON (a text, digits in a string, a negative number, a fraction and `true` for a duration; a fraction, a number beyond `i64` and a number in a string for a number; `1` and `"true"` for a flag; a number, an object and `null` for a text). Each part was broken on purpose one at a time, 15 times (the query without the tenant or without the name, `null` not handled, a string read with its quotes, a name without a default taken for a text, a wrong kind giving the default, the JSON or the name missing from the error, no row taken for an error, negative seconds wrapped round, fractions of seconds, a number, a duration or a flag of the wrong JSON accepted, a text of any JSON). 14 times a test went red at once; the last, digits in a string taken for a duration, needed the case `"7776000"`, which was added for it.

## Consequences

- **Every `get` with a known name is one query by the primary key.** Nothing is cached, so a change of a row is seen at once by every copy of the program.
- **A company that does not exist has the defaults.** The module cannot tell it from a company that changed nothing.
- **A duration has whole seconds.** A fraction of a second cannot be stored or read.
- **A row of a name that has no default any more,** e.g. of a setting a module dropped, stays in the table, and `get` refuses that name.
