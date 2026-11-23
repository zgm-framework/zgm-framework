# ADR-0008: `restore_default` deletes the row of a tenant without reading it and says whether there was one; a name without a default is refused

> **Status:** accepted
> **Date:** 2026-11-23
> **Applies to:** `zgm-settings` (`TenantScope::restore_default`, `repository::delete_value`, `tests/restore_default.rs`)

## Context

- **No row means the default** ([ADR-0002](0002-defaults-built-into-the-modules.md)), so going back to the default is losing the row. E.g. the company Nowak set 90 days, clicks "restore the default" on a screen of the program, and has the default 30 days again.
- **`zgm-limits` has `restore_default` for the rule of a tenant** ([ADR-0017 of `zgm-limits`](../../../zgm-limits/docs/adr/0017-tenant-rules-in-the-database.md)): it deletes the tenant's row and gives `true` only when something changed; a limit that is not on the builder's list is `UnknownLimit`; the row of a limit that is no longer in code stays.

## Options considered

The design followed `restore_default` of `zgm-limits`; the one question of its own was whether the value of the row is read first:

| Option | Pros | Cons |
|---|---|---|
| **A. Delete without reading** | one statement; a row of the wrong kind, which `get` refuses, can be removed with it | none found |
| **B. Read the row first, as `get` does** | the same checks as `get` | a row of the wrong kind could never be removed by the module |

## Decision

**A, and the rest as in `zgm-limits`** (accepted by the maintainer on 2026-11-23 with the design, after the question "I don't understand, which delete?" was answered with the rows of the table before and after: "ok").

```rust
in_nowak.restore_default(IamSetting::UserDeletionWindow).await?; // true: the row is gone, Nowak has 30 days again
in_nowak.restore_default(IamSetting::UserDeletionWindow).await?; // false: there was nothing to delete
```

- **`restore_default(name) -> Result<bool, SettingsError>`** is one `DELETE` by the primary key, `true` when a row was deleted.
- **A name no module gave a default for is `UnknownSetting`,** and nothing is deleted.
- **The value of the row is not read.**

## Why

- **`false` without a row** lets a program tell a real change from a repeated click, as `set` does ([ADR-0007](0007-set-a-setting-of-a-tenant.md)).
- **A name without a default is refused** like in `get` and `set`: a name the program got wrong deletes nothing, also when a row of that name is there.
- **Checked by tests,** on a database: the row goes, `restore_default` gives `true`, and `get` gives the default again; without a row it gives `false`; the row of another company and another setting of the same company stay; a name without a default is `UnknownSetting` and its row stays; a row of another kind than the default goes too, and `get` gives the default again. Each part was broken on purpose one at a time, 6 times (the delete without the tenant, without the name, always `true`, always `false`, a name without a default deleted, the value read before the delete), and every time a test went red.

## Consequences

- **The row of a setting that no module gives any more** cannot be removed through the module; it stays in the table, unread.
- **`restore_default` takes one connection for one statement,** with no transaction of its own.
