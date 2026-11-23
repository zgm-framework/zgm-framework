# ADR-0009: `list` gives every setting that has a default, in the order of the names, with the value in the tenant, the default and whether the tenant changed it; no pages

> **Status:** accepted
> **Date:** 2026-11-23
> **Applies to:** `zgm-settings` (`TenantScope::list`, `Setting`, `repository::select_values`, `tests/list.rs`)

## Context

- **A program shows the settings of a company on one screen,** e.g. "90 days (default 30)" with a button to restore the default next to what the company changed.
- **The defaults are in memory, the changes of a company in its rows** ([ADR-0002](0002-defaults-built-into-the-modules.md)), and a row is read as the kind of the default ([ADR-0006](0006-get-a-setting-of-a-tenant.md)).
- **`zgm-limits` has `get_rules` for one tenant:** every limit with the rule in force and an `overridden` flag; the row of a limit no longer in code is skipped ([ADR-0017 of `zgm-limits`](../../../zgm-limits/docs/adr/0017-tenant-rules-in-the-database.md)).
- **`list_users` in `zgm-iam` gives pages,** because a company may have 50,000 users, and refuses a whole page when a row holds a word of state it does not know.

## Options considered

The design was presented whole, each choice after a pattern of the repository:

| Question | Chosen | Other option, and why not |
|---|---|---|
| What an entry holds | the name, the value in the tenant, the default, and `overridden` | the value only: a screen could not show the default or what to restore |
| Which settings | every one a module gave a default for, in the order of the names | only those the tenant changed: a screen would merge them with `defaults()` itself |
| A row equal to the default | `overridden: true` | `false`: the company has a row of its own, which it keeps when the default changes ([ADR-0007](0007-set-a-setting-of-a-tenant.md)) |
| The row of a name without a default | left out, as in `get_rules` | an error: one dropped setting would break the screen of every company that had changed it |
| A row of the wrong kind | the whole list is `WrongKind`, as in `get` and in `list_users` | the default in its place: a wrong row, which only SQL by hand can write, would pass unnoticed |
| Pages | none | pages as in `list_users`: there are as many settings as the modules gave, tens, not 50,000 |

## Decision

**As above** (accepted by the maintainer on 2026-11-23 with the design: "ok").

```rust
let all = in_nowak.list().await?;
// [Setting { name: "iam.user_deletion_window", value: Duration(90 days), default: Duration(30 days), overridden: true }, …]
```

- **`list() -> Result<Vec<Setting>, SettingsError>`,** where `Setting { name, value, default, overridden }` has public fields and `PartialEq`.
- **One query reads all the rows of the tenant,** by the first column of the primary key; the module merges them with the defaults in memory.
- **The columns a value is read from are written once,** in the `stored_value_columns!` macro of `src/repository.rs`, for `get` and `list` alike.

## Why

- **An entry has what a screen needs,** with no second call.
- **The same columns for both queries,** so that `get` and `list` cannot read a row differently, e.g. a JSON `null`.
- **Checked by tests,** on a database: a company without rows has all the defaults, none overridden, in the order of the names; a changed setting has the value of the company and is overridden, the others have their defaults; a row equal to the default is overridden; the rows of another company do not count; the rows of names without a default are left out, also one whose name sorts before all the others; a row of the wrong kind makes the list `WrongKind` with its name and JSON; without defaults the list is empty, whatever the rows. Each part was broken on purpose one at a time, 10 times (the query without the tenant, an unchanged setting marked overridden, a changed one marked not, a row equal to the default not overridden, a wrong kind taken for the default, the rows of names without a default listed, the names in reverse order, the value of the row not used, a JSON `null` not handled and a string read with its quotes in the shared columns), and every time a test went red.

## Consequences

- **A program that shows the list does not call `get` for each setting.**
- **One wrong row hides the list of that company** until it is removed with `restore_default` ([ADR-0008](0008-restore-the-default.md)).
- **Every `list` reads all the rows of the company,** which are at most as many as its settings.
