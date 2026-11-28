# ADR-0010: `change_default` changes a default and its bounds in one statement, refusing another kind, a fraction of a second, a least above the most and a default outside its bounds; no function deletes a default

> **Status:** accepted
> **Date:** 2026-11-28
> **Applies to:** `zgm-settings` (`Settings::change_default`, `DefaultSetting`, `SettingsError::LeastAboveTheMost`, `repository::update_default`, `src/bounds.rs`, `tests/change_default.rs`, `tests/change_default_at_once.rs`)

## Context

- **A module writes its defaults once, at its first start, and checks nothing** ([ADR-0002](0002-defaults-built-into-the-modules.md), [ADR-0005](0005-settings-and-its-defaults.md)). A later number in its code does not reach a database that has the row, and only a function of the settings module changes the row after that.
- **A person changes a default on a screen of the program,** e.g. 14 days for deletion instead of 30, at least a week and at most 90 days, for every tenant that has not set its own.
- **`set` holds the value of a tenant against the bounds of the row** ([ADR-0007](0007-set-a-setting-of-a-tenant.md)), and `get` reads the value of a tenant as the kind of the row ([ADR-0006](0006-get-a-setting-of-a-tenant.md)). A least above the most in the row refuses every `set`; another kind in the row makes `get` fail for every tenant with a value of its own.
- **A bound in the table is `NULL` for no bound** ([ADR-0003](0003-the-tenant-settings-table.md)): a flag and a text have none, and a number may have only a least, e.g. at least 1 user and no most.
- **A module may drop a setting from its code;** the row of its default stays, and the database refuses to delete it while a tenant has a value of it ([ADR-0003](0003-the-tenant-settings-table.md), [ADR-0009](0009-list-the-settings-of-a-tenant.md)).

## Options considered

**What the function refuses,** e.g. for the window of 30 days, at least a day and at most a year:

| Option | Pros | Cons |
|---|---|---|
| **A. Nothing, as `add_default`** | one rule for both functions | 60 days at least and 59 at most refuses every `set`; a text for the window makes `get` fail for every tenant with a value of its own |
| **B. A least above the most** | `set` can always take some value | a text for the window passes; so does 30 days with at least 60, which a tenant without a value of its own gets and could not set itself |
| **C. A least above the most, and another kind** | `get` keeps reading the values of the tenants | 30 days with at least 60 passes |
| **D. A least above the most, another kind, and a default outside its bounds** | every default a person leaves is one a tenant could set | the two functions that write a default have other rules |

**A least equal to the most,** e.g. at least 7 users and at most 7: allowed, as a setting of one value, or refused.

**Bounds that are `NULL`:** kept as no bound, or made `NOT NULL`, which would give a flag and a text bounds they do not have.

**A function that deletes a default,** for a setting a module dropped: none; one that refuses while a tenant has a value of it, as the database does; one that deletes the values of the tenants with it.

## Decision

**What it refuses: D. A least equal to the most: allowed. Bounds: `NULL` stays no bound. Deleting: no function** (decided by the maintainer on 2026-11-28: on deleting, "a default without deletion"; on the order of the bounds, "the check min<max is ok", and "let min=max through, but not min>max"; on the bounds, first sure that they were `NOT NULL`, and then, shown a flag and a number with a least only, "ah, then yes, let it be nullable there"; on the kind and the bounds of the default, "then A").

```rust
// 14 days instead of 30, at least a week, at most 90 days.
let window = DefaultSetting::Duration { value: two_weeks, min: Some(a_week), max: Some(ninety_days) };
settings.change_default(IamSetting::UserDeletionWindow, window.clone()).await?; // true
settings.change_default(IamSetting::UserDeletionWindow, window).await?;         // false
let crossed = DefaultSetting::Duration { value: thirty_days, min: Some(sixty_days), max: Some(fifty_nine_days) };
settings.change_default(IamSetting::UserDeletionWindow, crossed).await;
// Err(LeastAboveTheMost { name: "iam.user_deletion_window", min: Duration(60 days), max: Duration(59 days) })
```

- **`Settings::change_default(name, DefaultSetting) -> Result<bool, SettingsError>`,** with the `DefaultSetting` of `add_default`. `true` when the row changed; the same default and bounds again write nothing, give `false`, and `updated_at` stays.
- **It reads the row of the default first,** by its key: no row is `UnknownSetting`. The kind of the row and the kind given must be the same word, else `WrongKind` with the default given as Rust shows it, e.g. `Text("30 days")`; a row of a kind the module does not know is refused so. The value of the row is not read, so a default the module cannot read, which only SQL by hand can write, is written over by a change of its kind.
- **Then it checks, in this order:** a duration or a bound with a fraction of a second (`FractionOfASecond`); a least above the most (`LeastAboveTheMost`, both as values of the kind of the setting); a default below its least or above its most (`BelowTheLeast`, `AboveTheMost`). A bound itself is allowed, and so is a least equal to the most. `None` is no bound; a flag or a text has none.
- **A duration and its bounds are compared in whole seconds,** more than a `bigint` holds as the most it holds, the way the table keeps a bound.
- **Then one statement writes:** `UPDATE … SET value, min, max … WHERE name = $1 AND (value, min, max) IS DISTINCT FROM (…)`. A text goes as plain text, turned into a JSON string by `to_jsonb`, as in `set`.
- **The values of the tenants are not touched.** A tenant without a value of its own gets the new default at its next `get`; a value outside the new bounds stays and is given as it is; the next `set` of every tenant is held against the new bounds.
- **The checks of the bounds are written once,** in `src/bounds.rs`, for `set` and `change_default` alike.
- **No function deletes a default.**

## Why

- **The checks:** a person types the numbers on a screen, and a mistake there would stop every tenant from setting the value, stop `get` for the tenants with a value of their own, or give the tenants a default they could not set themselves. `add_default` trusts the code of a module, which its own tests check ([ADR-0005](0005-settings-and-its-defaults.md)); a screen has no tests.
- **A least equal to the most:** a setting held at one value for every tenant is a choice a person may make.
- **`NULL` for no bound:** a flag and a text have no bounds, and a number with a least only would need a most it does not have.
- **A fraction of a second refused, not cut off:** the table keeps whole seconds, so `1.5 s` would be kept as `1 s`, as in `set` ([ADR-0007](0007-set-a-setting-of-a-tenant.md)).
- **No function to delete:** the maintainer's decision. A setting a module dropped stays on the list of every tenant ([ADR-0009](0009-list-the-settings-of-a-tenant.md)); the values the tenants set of it go with `restore_default` ([ADR-0008](0008-restore-the-default.md)).
- **The kind is not looked at again in the write:** only the module writes the table ([ADR-0002](0002-defaults-built-into-the-modules.md)), and no function of it changes a kind.
- **`IS DISTINCT FROM` for the row of three:** a repeated click changes nothing, as in `set`, and a change of a bound alone is a change.
- **Checked by tests** in `tests/change_default.rs`, on a database: a new default and new bounds are written and give `true`; a default of every kind is changed, among them `i64::MIN` as a least and a text with quotes, a backslash, a line break and Polish letters; a change of the bounds alone is a change, also to no bound; the same default and bounds again give `false` for every kind and leave `updated_at`, and a change moves it; the other defaults stay; a name without a default is `UnknownSetting`; six defaults of another kind are `WrongKind` with the value given; a least above the most is `LeastAboveTheMost` with both bounds, and a least equal to the most is taken; defaults below the least or above the most are refused with the bound, and a default on a bound is taken; a bound that is `None` holds nothing; seconds above a `bigint` are held as the most it holds; a fraction of a second in the default or in either bound is refused; in every refusal nothing changes. A tenant without a value of its own gets the new default at once; a tenant keeps a value outside the new bounds, its next `set` is held against them, and `list` shows them; a later start of the module leaves the change; a default this program did not give is changed too; a default the module cannot read is mended when its kind is given, and one of a kind the module does not know is refused; the values of the tenants stay; a database that does not answer is an error.
- **Checked under load** in `tests/change_default_at_once.rs`: 50 changes from 16 copies of the program at once all succeed, and one of them stays; the same change from 16 copies, 5 times each, is one change; 16 tenants list 200 times while the default and its bounds change, and each sees the old three or the new, never mixed, and never the old again after the new; 16 copies start 20 times each while a person changes the default, and the change stays.
- **Seen to fail first:** 23 of the 24 tests went red on a `change_default` that wrote nothing; the one left checks that the values of the tenants stay. Then `change_default` was broken on purpose, one change at a time, 17 times (no check of the kind, an unknown name taken as no change, no check of a fraction or one in the default or in the bounds only, a least equal to the most refused, no check of the order of the bounds, the bounds swapped in the error, no check of the default within its bounds or one against the bounds in the database, the checks after the write, the same again written anyway, only the default compared, the bounds not written or swapped in the write, always `true`, every default changed), and every time a test went red. Not measured: a change from a screen is a click, not a call on every request.

## Consequences

- **Every `change_default` is two statements on one connection,** with no transaction: the read of the kind and the write.
- **Who may change a default is for the program to decide.** The module does not ask, and keeps no record of who changed it.
- **`add_default` and `change_default` refuse different things:** the numbers a module gives in its code are written as they are, and the same numbers typed on a screen may be refused.
- **A default a module dropped stays in the database for good,** in `defaults()` and on every list.
