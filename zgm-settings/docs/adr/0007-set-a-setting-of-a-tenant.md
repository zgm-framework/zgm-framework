# ADR-0007: `set` reads the kind and the bounds of the default from the database, refuses a value outside the bounds in force, also the value the tenant has, and writes or changes the row of the tenant in one statement, saying whether anything changed

> **Status:** accepted
> **Date:** 2026-11-26
> **Applies to:** `zgm-settings` (`TenantScope::set`, `repository::select_default`, `repository::upsert_value`, `Value::to_json`, `Value::as_bounded`, `SettingsError`, `tests/set.rs`, `tests/set_at_once.rs`)

## Context

- **A tenant changes a setting,** e.g. on a screen of the program the tenant Nowak keeps users waiting for deletion 90 days instead of the default 30.
- **The row of the default has the kind and the bounds,** e.g. at least a day and at most a year, and they may be changed in the database while the program runs ([ADR-0002](0002-defaults-built-into-the-modules.md), [ADR-0003](0003-the-tenant-settings-table.md)). Nothing checks the bounds a module gives ([ADR-0005](0005-settings-and-its-defaults.md)).
- **A row of a tenant holds plain JSON, read as the kind of the default** ([ADR-0006](0006-get-a-setting-of-a-tenant.md)): a duration in whole seconds, a number, `true` or `false`, a string. `get` gives the value of a tenant as it is, also outside bounds lowered after it was set.
- **`zgm-limits` changes the rule of a tenant with `override_rule`** ([ADR-0017 of `zgm-limits`](../../../zgm-limits/docs/adr/0017-tenant-rules-in-the-database.md)): saving the same rule again changes nothing and gives `false`, and a rule equal to the default is a row too. What a rule may be changed to is bounded by numbers written over the limit, and `override_rule` refuses a rule outside them ([ADR-0018 of `zgm-limits`](../../../zgm-limits/docs/adr/0018-override-bounds-in-strum-attributes.md)).

## Options considered

**The same value again, set while it lies outside bounds lowered since,** e.g. Nowak has 90 days from when the most was 365, the most is now 60, and Nowak saves the screen without a change:

| Option | Pros | Cons |
|---|---|---|
| **A. Refused, as any value above the most** | every `set` is held against the bounds in force; the tenant learns that its value no longer fits | saving an unchanged screen gives an error |
| **B. Taken as no change, `false`** | saving an unchanged screen never fails | a value outside the bounds passes `set` without a word |

**How the default is read for the check:**

| Option | Pros | Cons |
|---|---|---|
| **A. A statement of its own before the write** | the checks are in Rust, in one place, with an error for each | two statements: measured 0.15 ms more for one `set` |
| **B. One statement that writes only when the kind and the bounds fit, and gives the row of the default back to tell why not** | one statement | every check twice, in SQL to write and in Rust to say why not |

## Decision

**The same value again: A** (decided by the maintainer on 2026-11-26: "A", after the first decision on bounds that do not reach back, "when somebody changes the max, it guards only new changes"). **Reading the default: A,** as it was designed, since `set` is a click of a person on a screen, not a call on every request.

```rust
// The default: 30 days, at least a day, at most a year.
in_nowak.set(IamSetting::UserDeletionWindow, Value::Duration(ninety_days)).await?; // true
in_nowak.set(IamSetting::UserDeletionWindow, Value::Duration(ninety_days)).await?; // false
in_nowak.set(IamSetting::UserDeletionWindow, Value::Duration(four_hundred_days)).await?;
// Err(AboveTheMost { name: "iam.user_deletion_window", max: Duration(365 days) })
```

- **`set(name, value) -> Result<bool, SettingsError>` reads the row of the default first,** by its key, at every call: no row is `UnknownSetting`, and a row the module cannot read is `UnreadableDefault`.
- **Then it checks, in this order:** the value is of the kind of the default (else `WrongKind`, with the value as Rust shows it, e.g. `Text("90 days")`); a duration has no fraction of a second (else `FractionOfASecond`); a duration or a number is not below the least (else `BelowTheLeast`) nor above the most (else `AboveTheMost`). The bounds themselves are allowed. The error has the bound as a `Value` of the kind of the setting, e.g. `Duration(365 days)`, for a screen to show. A flag or a text has no bounds, also when its row has some.
- **A duration is held against its bounds in whole seconds,** and more seconds than a `bigint` holds as the most it holds, the way its bounds are kept. A bound of a duration below zero, which only SQL by hand can write, is no time.
- **Then one statement writes the row of the tenant or changes the value it had:** `INSERT … ON CONFLICT (tenant_id, name) DO UPDATE SET value = excluded.value WHERE tenant_settings.value IS DISTINCT FROM excluded.value`. It gives `true` when a row was written or changed; the same value again writes nothing, gives `false`, and `updated_at` stays.
- **A value equal to the default is a row too.**
- **In every refusal nothing is written.**
- **A number or a flag goes to the table as the text of its JSON,** which the database reads as `jsonb`; **a text goes as plain text,** which the database turns into a JSON string with `to_jsonb`, so the module escapes nothing itself.

## Why

- **The bounds in force for every `set`:** a bound guards what is set from the moment it holds, and saving is setting, also of the same number. What a tenant set before keeps holding for `get` ([ADR-0006](0006-get-a-setting-of-a-tenant.md)).
- **The kind and the bounds from the database:** they are where they can change, and a copy of the program that gave no default checks the same as the one that did.
- **`false` for the same value** lets a program tell a real change from a repeated click, and `updated_at` says when the value last changed, not when it was last saved.
- **A row equal to the default:** a tenant that chose 30 days keeps 30 when the default is later changed to 14.
- **A fraction of a second is refused, not cut off:** the table keeps whole seconds, so `1.5 s` would be kept as `1 s`, another value than the one the tenant gave.
- **`to_jsonb` for a text:** the database escapes quotes, backslashes and line breaks, and the module needs no JSON crate.
- **Checked by tests** in `tests/set.rs`, on a database: a value of every kind set is what `get` gives, among them 0 seconds, `u64::MAX` seconds, `i64::MIN`, `i64::MAX`, an empty text and a text with quotes, a backslash, a line break and Polish letters; values lie in the table as plain JSON; a new value gives `true`; another value replaces the one before and moves `updated_at`; the same value again gives `false` and leaves `updated_at`; the value of another tenant stays; a value equal to the default is a row; a name without a default is `UnknownSetting`; eight values of another kind are `WrongKind`; 1.5 s and 1 ns are `FractionOfASecond`. And with bounds: a value within them is set, the least and the most themselves too; values below the least (0 days, a second short of a day, 0 and `i64::MIN` users) are `BelowTheLeast` and values above the most (400 days, a second past a year, `u64::MAX` seconds, 1001 and `i64::MAX` users) are `AboveTheMost`, each with its bound, and the row there stays; a bound that is `None` holds nothing; the same value again above a most lowered since is refused and the tenant keeps its value; a change of the bounds holds for the next `set`; a least above the most refuses every value; bounds on a flag or a text hold nothing; the kind and the bounds come from the database also for a setting this program did not give; a default the module cannot read refuses; a bound of a duration below zero is no time.
- **Checked under load** in `tests/set_at_once.rs`: 100 tenants set at once and each keeps its own value; one tenant sets 50 values at once, none fails, and one of them stays; 16 tenants set 200 times each while the most is lowered once a quarter of the sets are done, and every answer is taken until the first refusal for the new most and refused after it, each tenant keeping the last value taken.
- **Seen to fail first:** 7 of the new tests went red on the `set` that took the kind from memory and checked no bounds. 3 passed at once, as the old `set` refused no value for its bounds: a value within them, a bound that is `None`, bounds on a flag or a text. Then `set` was broken on purpose, one change at a time, 15 times (the least or the most itself refused, no check of the least or of the most, the bound in the error off by one, the bounds not looked at, swapped in Rust or in the query, checked after the write, the kind from memory, seconds above a `bigint` wrapped round, a duration held in milliseconds, a flag held against bounds, a bound below zero shown as a second, an unreadable default taken as a text), and every time a test went red.
- **Measured** on one laptop with PostgreSQL 18 in Docker, 1,000 tenants, every call another value: one `set` at a time took 0.32 ms at the median before the check of the bounds and 0.46–0.52 ms after, with its second statement; 16 at once gave 5,900–6,400 calls a second before and 4,000–5,500 after.

## Consequences

- **Every `set` is two statements on one connection,** with no transaction: the read of the default and the write.
- **A value is held against the bounds as they were when it was read.** A change of the bounds that comes between the read and the write does not reach that `set`, as it would not reach a value set a moment before.
- **A mistake in the bounds a module gave refuses values.** A least above the most refuses every value, until the default is changed through the function of the module.
- **Two tenants writing at once do not disturb each other,** and two writes of one tenant at once end with one of the two values, because the statement locks the row it changes.
