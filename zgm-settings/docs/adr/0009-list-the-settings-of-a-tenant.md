# ADR-0009: `list` gives every default in the database, in the order of the bytes of the names, with the value in the tenant, the default, whether the tenant changed it and the bounds; one statement, no pages

> **Status:** accepted
> **Date:** 2026-11-26
> **Applies to:** `zgm-settings` (`TenantScope::list`, `Setting`, `Settings::defaults`, `repository::select_settings`, `repository::select_defaults`, `tests/list.rs`)

## Context

- **A program shows the settings of a tenant on one screen,** e.g. "90 days (default 30, from 1 to 365 days)" with a button to restore the default next to what the tenant changed.
- **The defaults and their bounds are in the database, and so are the changes of a tenant** ([ADR-0002](0002-defaults-built-into-the-modules.md), [ADR-0003](0003-the-tenant-settings-table.md)). `get`, `set` and `restore_default` read them there at every call ([ADR-0006](0006-get-a-setting-of-a-tenant.md)).
- **A module may drop a setting from its code,** e.g. a later version of `zgm-iam` without `iam.self_sign_up`; the row of its default stays in the database.
- **`zgm-limits` has `get_rules` for one tenant:** every limit with the rule in force and an `overridden` flag ([ADR-0017 of `zgm-limits`](../../../zgm-limits/docs/adr/0017-tenant-rules-in-the-database.md)).
- **`list_users` in `zgm-iam` gives pages,** because a tenant may have 50,000 users, and refuses a whole page when a row holds a word of state it does not know.

## Options considered

The first design was presented whole, each choice after a pattern of the repository:

| Question | Chosen | Other option, and why not |
|---|---|---|
| What an entry holds | the name, the value in the tenant, the default, `overridden`, and the bounds | the value only: a screen could not show the default, the bounds or what to restore |
| A row equal to the default | `overridden: true` | `false`: the tenant has a row of its own, which it keeps when the default changes ([ADR-0007](0007-set-a-setting-of-a-tenant.md)) |
| A row of the wrong kind | the whole list is an error, as in `get` and in `list_users` | the default in its place: a wrong row, which only SQL by hand can write, would pass unnoticed |
| Pages | none | pages as in `list_users`: there are as many settings as the modules gave, tens, not 50,000 |

**Which settings, now that the defaults are in the database,** e.g. with `iam.self_sign_up` dropped from the code of `zgm-iam`:

| Option | Pros | Cons |
|---|---|---|
| **A. Every default in the database** | one place the list comes from, as for every other function; every copy of the program lists the same | a dropped setting is listed until its default is deleted |
| **B. Only the settings this copy of the program gave when it started** | a dropped setting is not listed | the names kept in memory again; a copy that gave none lists none; copies of two versions list differently |

## Decision

**As in the first table, and which: A** (the first design accepted by the maintainer on 2026-11-23: "ok"; on 2026-11-26 on which settings: "well, delete it then, everything goes through the settings module now, right? go on"). A setting a module dropped is not left out of the list; on 2026-11-28 the maintainer decided that no function deletes a default ([ADR-0010](0010-change-a-default.md)), so it stays there. With nothing left that reads it, the memory of the defaults was deleted as well ([ADR-0005](0005-settings-and-its-defaults.md)).

```rust
let all = in_nowak.list().await?;
// [Setting { name: "iam.user_deletion_window", value: Duration(90 days), default: Duration(30 days),
//            overridden: true, min: Some(Duration(1 day)), max: Some(Duration(365 days)) }, …]
```

- **`list() -> Result<Vec<Setting>, SettingsError>`,** where `Setting { name, value, default, overridden, min, max }` has public fields and `PartialEq`. A bound is a `Value` of the kind of the setting, or `None` for no bound; a flag or a text has none, also when its row has some.
- **One statement reads every default and the rows of the tenant:** `default_settings d LEFT JOIN tenant_settings t ON t.tenant_id = $1 AND t.name = d.name ORDER BY d.name COLLATE "C"`.
- **The order is that of the bytes of the names,** as Rust orders text and as `Settings::defaults` gives them, not that of the collation of the database: in `en_US.utf8`, `iam.a` comes before `iam.B`.
- **The value of a tenant is given as it is,** also outside the bounds, as in `get`.
- **A row the module cannot read makes the whole list an error:** `UnreadableDefault` for a default, `WrongKind` for a value of the tenant.
- **`Settings::defaults()` is `async` and reads every default from the database,** without a tenant.
- **The columns a value is read from are written once,** in the `stored_value_columns!` macro of `src/repository.rs`, and those of a row of a default in `default_row_columns!`, for every query alike.

## Why

- **An entry has what a screen needs,** with no second call.
- **Every default in the database:** the maintainer wants everything to go through the database and the settings module, and a list that hides a setting would show a different screen from the copy that answers.
- **`COLLATE "C"`:** without it the list would come in the order of the database, which depends on how the database was created, and would differ from the order of `defaults()`. Checked on the database of the tests, which is `en_US.utf8`: it orders `iam.a iam.B iam._x iam.Z`, and `COLLATE "C"` `iam.B iam.Z iam._x iam.a`.
- **The same columns for every query,** so that `get` and `list` cannot read a row differently, e.g. a JSON `null`.
- **Checked by tests** in `tests/list.rs`, on a database: a tenant without rows has all the defaults, none overridden, in the order of the names; a changed setting has the value of the tenant and is overridden, the others have their defaults; a row equal to the default is overridden; the rows of another tenant do not count; a setting this program did not give is listed too, also one whose name sorts before all the others; the order is that of the bytes, the same as of `defaults()`; each setting has the bounds of its default, a flag none, also with bounds written by hand; a change of a default or its bounds is seen by the next list; a value above a most lowered since is listed as it is; a default the module cannot read and a row of the wrong kind make the list an error with the name and the JSON; without defaults in the database the list is empty; a copy of the program that gave no default lists those of the database.
- **Seen to fail first:** 7 of the new tests went red on the `list` that took the defaults from memory, and 4 of the changed tests of the defaults. Then `list` and `defaults` were broken on purpose, one change at a time, 14 times (the order of the collation of the database, no order, an inner join, the rows of any tenant, the rows of any setting, bounds shown on a flag, the bounds swapped, no bounds, never changed, always changed, the value always the default, the text instead of the JSON in the error, an unreadable default left out of `defaults()`, `add_default` always `true`). 13 times a test went red. The text in the error passed, as the test had a row of `1`, whose JSON and text are the same; it now has the string `"yes"`, and the change goes red.
- **Measured** on one laptop with PostgreSQL 18 in Docker, 1,000 tenants, half of the settings of each changed, the code before and after taken in turn: with 4 defaults one `list` at a time took 0.30 ms at the median before and 0.33 ms after; with 50, 0.32–0.39 ms before and 0.39–0.45 ms after, the defaults now read from the database at every call. 16 at once with 50 defaults gave 6,000–7,900 calls a second before; after, the first two runs gave 3,900 and 1,100, and three runs taken in turn with the code before gave 5,700–7,000 after against 6,200–7,900 before. The plan of the statement reads the rows of the tenant through one search of the primary key, 27 pages, as before.

## Consequences

- **A program that shows the list does not call `get` for each setting.**
- **A setting a module dropped stays on the list,** as no function deletes its default ([ADR-0010](0010-change-a-default.md)).
- **One wrong row hides the list of that tenant:** a row of the tenant until it is removed with `restore_default` ([ADR-0008](0008-restore-the-default.md)), a row of a default for every tenant until it is mended.
- **Every `list` reads every default and the rows of the tenant,** which are at most as many as its settings.
