# ADR-0008: `restore_default` deletes the row of a tenant without reading it, in one statement that also finds the default in the database, and says whether there was a row; a name without a default is refused

> **Status:** accepted
> **Date:** 2026-11-26
> **Applies to:** `zgm-settings` (`TenantScope::restore_default`, `repository::delete_value`, `tests/restore_default.rs`, `tests/restore_default_at_once.rs`)

## Context

- **No row of a tenant means the default** ([ADR-0002](0002-defaults-built-into-the-modules.md)), so going back to the default is losing the row. E.g. the tenant Nowak set 90 days, clicks "restore the default" on a screen of the program, and has the default again.
- **The default is in the database,** where it may change while the program runs, and `get` and `set` know a setting by its row there ([ADR-0006](0006-get-a-setting-of-a-tenant.md), [ADR-0007](0007-set-a-setting-of-a-tenant.md)). A row of a tenant cannot exist without the row of its default ([ADR-0003](0003-the-tenant-settings-table.md)).
- **`zgm-limits` has `restore_default` for the rule of a tenant** ([ADR-0017 of `zgm-limits`](../../../zgm-limits/docs/adr/0017-tenant-rules-in-the-database.md)): it deletes the tenant's row and gives `true` only when something changed; a limit that is not on the builder's list is `UnknownLimit`.

## Options considered

The design followed `restore_default` of `zgm-limits`; the one question of its own was whether the value of the row is read first:

| Option | Pros | Cons |
|---|---|---|
| **A. Delete without reading** | one statement; a row of the wrong kind, which `get` refuses, can be removed with it | none found |
| **B. Read the row first, as `get` does** | the same checks as `get` | a row of the wrong kind could never be removed by the module |

When the defaults moved into the database, the setting became known by the row of its default, as for `get` and `set`; the default itself is not read either.

## Decision

**A, and the rest as in `zgm-limits`** (accepted by the maintainer on 2026-11-23 with the design, after the question "I don't understand, which delete?" was answered with the rows of the table before and after: "ok"; and on 2026-11-26 that the defaults are read from the database every time, "no, it is to take it from the database every time, without a cache, right?").

```rust
in_nowak.restore_default(IamSetting::UserDeletionWindow).await?; // true: the row is gone, Nowak has the default again
in_nowak.restore_default(IamSetting::UserDeletionWindow).await?; // false: there was nothing to delete
```

- **`restore_default(name) -> Result<bool, SettingsError>` is one statement:** a `DELETE` of the row of the tenant by its key, in a `WITH`, and a `SELECT` of whether the setting has a default and whether a row was deleted. `true` when a row was deleted.
- **A name without a default in the database is `UnknownSetting`.** There is no row of the tenant to delete then.
- **Neither the row of the tenant nor the default is read,** so a row goes also when its value is of another kind or the default cannot be read.

## Why

- **`false` without a row** lets a program tell a real change from a repeated click, as `set` does ([ADR-0007](0007-set-a-setting-of-a-tenant.md)).
- **Known by the default in the database,** like in `get` and `set`, so a copy of the program that gave no default restores too, and so does a program for a setting an earlier version of a module left.
- **One statement:** the check of the default costs no round trip of its own. Measured before and after, below.
- **Checked by tests** in `tests/restore_default.rs`, on a database: the row goes, `restore_default` gives `true`, and `get` gives the default again; without a row it gives `false`; the row of another tenant and another setting of the same tenant stay; a name without a default in the database is `UnknownSetting`; a setting this program did not give is restored too; a copy of the program that gave no default restores too; the tenant gets the default the database has now, after it changed; a row goes also when the default cannot be read; a row of another kind than the default goes too.
- **Checked under load** in `tests/restore_default_at_once.rs`: 100 tenants restore at once and each loses its row; one tenant restores 50 times at once and exactly one call finds the row; 100 sets and 100 restores of one tenant at once all succeed, and the tenant ends with its value or the default.
- **Seen to fail first:** 3 of the new tests went red on the `restore_default` that knew the settings from memory (the setting this program did not give, the copy that gave no default, the default that cannot be read). The 2 other new ones passed at once: a name without a default was refused before too, and `get` already read the default from the database. Then `restore_default` was broken on purpose, one change at a time, 9 times (every name known, always a row, the row of any tenant, every row of the tenant, the two answers swapped, an unknown name taken as known, a known one refused, always `true`, the settings known from memory), and every time a test went red.
- **Measured** on one laptop with PostgreSQL 18 in Docker, 1,000 tenants, every call after a `set` that is not timed: one `restore_default` took 0.33–0.61 ms at the median before, 0.35–0.37 ms in a quieter second run, and 0.31–0.36 ms after.

## Consequences

- **The row of a tenant of a setting that a module no longer gives** can be removed as long as the default of the setting is in the database.
- **`restore_default` takes one connection for one statement,** with no transaction of its own.
