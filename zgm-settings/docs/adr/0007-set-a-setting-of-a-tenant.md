# ADR-0007: `set` writes or changes the row of a tenant in one statement and says whether anything changed; it checks the kind and whole seconds, and no range

> **Status:** accepted
> **Date:** 2026-11-23
> **Applies to:** `zgm-settings` (`TenantScope::set`, `repository::upsert_value`, `Value::to_json`, `SettingsError`, `tests/set.rs`)

## Context

- **A company changes a setting,** e.g. on a screen of the program the company Nowak keeps users waiting for deletion 90 days instead of the default 30.
- **A row holds plain JSON, read as the kind of the default** ([ADR-0006](0006-get-a-setting-of-a-tenant.md)): a duration in whole seconds, a number, `true` or `false`, a string.
- **`zgm-limits` changes the rule of a tenant with `override_rule`** ([ADR-0017 of `zgm-limits`](../../../zgm-limits/docs/adr/0017-tenant-rules-in-the-database.md)): saving the same rule again changes nothing and gives `false`, and a rule equal to the default is a row too, so the tenant keeps it when the default in code changes. What a rule may be changed to is bounded by numbers the product writes over the limit ([ADR-0018 of `zgm-limits`](../../../zgm-limits/docs/adr/0018-override-bounds-in-strum-attributes.md)).

## Options considered

**Whether `set` checks a range,** e.g. refuses 0 days or 100 years:

| Option | Pros | Cons |
|---|---|---|
| **A. No range; `set` checks only the kind** | one function in this step; the module works correctly without ranges | a program that does not check the value first lets a company set 0 days |
| **B. Ranges given with the default, e.g. 30 days from 1 to 365, and checked by `set`** | a wrong value is refused at once, whatever the program | twice the step: ranges for durations and numbers, their check at startup, their tests |

The rest of the design followed `override_rule`, with one choice of its own, marked below.

## Decision

**A** (decided by the maintainer on 2026-11-23: "well, without bounds"; and: "the settings module itself does not know which value is reasonable for which key, right?"). The rest was accepted with the design ("ok").

```rust
in_nowak.set(IamSetting::UserDeletionWindow, Value::Duration(ninety_days)).await?; // true
in_nowak.set(IamSetting::UserDeletionWindow, Value::Duration(ninety_days)).await?; // false
```

- **`set(name, value) -> Result<bool, SettingsError>`** writes the row of the tenant, or changes the value it had, in one statement: `INSERT … ON CONFLICT (tenant_id, name) DO UPDATE SET value = excluded.value WHERE tenant_settings.value IS DISTINCT FROM excluded.value`. It gives `true` when a row was written or changed.
- **The same value again writes nothing,** so `set` gives `false` and `updated_at` stays.
- **A value equal to the default is a row too.**
- **Before the database, `set` checks:** that a module gave a default for the name (else `UnknownSetting`); that the value is of the kind of the default (else `WrongKind`, with the value as Rust shows it, e.g. `Text("90 days")`); and that a duration has no fraction of a second (else `FractionOfASecond`, the one choice without a pattern in the repository). In each case nothing is written.
- **A number or a flag goes to the table as the text of its JSON,** which the database reads as `jsonb`; **a text goes as plain text,** which the database turns into a JSON string with `to_jsonb`, so the module escapes nothing itself.
- **No range is checked.** If ranges come, the module of a setting gives them with its default and this module only enforces them, as in `zgm-limits`; whether they are needed is decided after the list of all the settings of a company.

## Why

- **A range is not the settings module's to know:** only the module of a setting, e.g. `zgm-iam`, knows that 0 days make no sense, and only the program knows what its users may choose.
- **`false` for the same value** lets a program tell a real change from a repeated click, and `updated_at` says when the value last changed, not when it was last saved.
- **A row equal to the default:** a company that chose 30 days keeps 30 when the program later changes the default to 14.
- **A fraction of a second is refused, not cut off:** the table keeps whole seconds, so `1.5 s` would be kept as `1 s`, another value than the one given, and nobody would know.
- **`to_jsonb` for a text:** the database escapes quotes, backslashes and line breaks, and the module needs no JSON crate; a number or a flag has no characters to escape.
- **Checked by tests,** on a database: a value of every kind set is what `get` gives, among them 0 seconds, `u64::MAX` seconds, `i64::MIN`, `i64::MAX`, an empty text and a text with quotes, a backslash, a line break and Polish letters; values lie in the table as plain JSON (`7776000`, `-5`, `true`, `"a \"b\""`); a new value gives `true`; another value replaces the one before, gives `true` and moves `updated_at`; the same value again gives `false` and leaves `updated_at`; the value of another company stays; a value equal to the default is a row; a name without a default is `UnknownSetting` and writes nothing; eight values of another kind are `WrongKind` with the name and the value, and the rows that were there stay; 1.5 s and 1 ns are `FractionOfASecond` and write nothing. Each part was broken on purpose one at a time, 13 times (the same value written again, always a change, a new value not replacing the old, no check of the kind, fractions cut off, a name without a default written, a text not turned into JSON, a duration in milliseconds, the row written under another tenant, a value equal to the default not kept, the kind compared by value, the error without the value, a flag written as a text), and every time a test went red.

## Consequences

- **A program checks the range of a value before `set`,** until ranges exist.
- **Two companies writing at once do not disturb each other,** and two writes of one company at once end with one of the two values, because the statement locks the row it changes.
- **`set` takes one connection for one statement,** with no transaction of its own.
