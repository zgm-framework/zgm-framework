# ADR-0003: `zgm_settings.tenant_settings` has one row per tenant and setting: the name with the prefix of its module, the value as `jsonb`, the key `(tenant_id, name)` and no foreign key

> **Status:** accepted
> **Date:** 2026-11-23
> **Applies to:** `zgm-settings` (`migrations/20261123105422_create_tenant_settings.sql`, `tests/tenant_settings_table.rs`)

## Context

- **The table holds only the settings a company changed** ([ADR-0002](0002-defaults-built-into-the-modules.md)): e.g. the company Nowak keeps users waiting for deletion 90 days, and the company Kowalski, without a row, has the default.
- **Settings come from several modules,** and each names its settings with an enum.
- **The settings will be of several kinds:** a number of seconds, yes or no, a text, or several numbers at once, such as "10 attempts per hour".
- **The tenants live in `zgm-iam`,** and the modules do not depend on each other.
- **`zgm-limits` has a table of the same purpose,** `tenant_rules`: the key `(tenant_id, limit_name)`, the rule in `jsonb`, two dates and a trigger for `updated_at`.

## Options considered

**The name of a setting:**

| Option | Pros | Cons |
|---|---|---|
| **A. The prefix of the module in one column:** `iam.deletion_window` | one column; two modules cannot give the same name; the row says whose setting it is | a longer text in every row |
| **B. A column for the module:** (`iam`, `deletion_window`) | the module can be queried apart | a key of three columns |
| **C. No prefix; the same name given twice stops the program at startup** | the shortest names | the names must differ across all modules at once |

**The value:**

| Option | Pros | Cons |
|---|---|---|
| **A. `jsonb`** | one column for every kind of setting; readable in the database; the same as `tenant_rules` | the database checks only that it is JSON, not that it is a number of seconds |
| **B. `text`** | no JSON | the database checks nothing; each module writes several numbers its own way |
| **C. `bigint`** | the database checks that it is a number | numbers only; another kind of setting means changing the table |

**The name of the table:** `tenant_settings`, `settings`, `overrides` (what the table holds: only changes against the defaults), `tenant_overrides`.

## Decision

**Name: A. Value: A. Table: `tenant_settings`** (decided by the maintainer on 2026-11-23: "well, let it be A", "A", and on the table "or actually let it be `tenant_settings`"; the name comes from the module's enum through `strum`: "and it has to have strum, which turns that name in the enum into a string").

```sql
CREATE TABLE zgm_settings.tenant_settings (
    tenant_id   uuid        NOT NULL,
    name        text        NOT NULL,   -- e.g. 'iam.user_deletion_window'
    value       jsonb       NOT NULL,   -- e.g. 7776000
    created_at  timestamptz NOT NULL DEFAULT statement_timestamp(),
    updated_at  timestamptz NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT tenant_settings_pkey PRIMARY KEY (tenant_id, name)
);
```

- **The name is made by `strum::IntoStaticStr`** with `serialize_all = "snake_case"` and the module's `prefix`, e.g. `prefix = "iam."`.
- **The key is the tenant first and then the name,** with no id of its own. The trigger `touch_tenant_settings` sets `updated_at` on every change.
- **There is no foreign key to the tenants.** `tenant_id` is a plain UUID, as in `zgm-limits`.
- **Going back to the default deletes the row.** The table has no soft deletion.
- **The database checks no name and no value beyond JSON.** Only the module's code writes the table, so the code that writes and reads a setting checks it.

## Why

- **The prefix:** two modules may both have a `DeletionWindow`, e.g. the days a user waits in `zgm-iam` and the days old sessions are kept in a module of sign-in; without a prefix both would be `deletion_window`. Checked on `strum` 0.28.0: with `serialize_all = "snake_case", prefix = "iam."`, `IamSetting::UserDeletionWindow` gives `iam.user_deletion_window`.
- **`jsonb`:** one column holds every kind of setting, and the module that reads a setting is the one that knows what it must be.
- **The key `(tenant_id, name)`:** a row is always found by its company and its name, and nothing refers to a row, so an id of its own would be one more index for nothing. The tenant first, so that the same index finds one setting of a company and all of its settings. The key also keeps a company to one value per setting.
- **No foreign key:** the tenants are in another module, and a key across modules would tie them together.
- **Checked by tests,** with SQL by hand: a new row gets the moment of its statement as both dates, also late in a transaction; a change sets `updated_at` to the moment of the change, whatever the statement says; the same setting twice in one company is refused and the first value stays; the same setting in two companies and two settings of one company are two rows each; a row without a company, a name or a value is refused; `'90 days'` is refused as no JSON; a number, `true`, a text and an object are kept as written; a row can be deleted; the key is `tenant_id`, then `name`. Each of these, and the migration history in the module's own schema, was broken on purpose one at a time (19 changes, among them the key in another order or of one column, the value as `text`, `bigint` or `json`, the dates from `now()`, the trigger with `now()`, after the update or missing, the table in `public`, deletes refused). 18 times a test went red. The one change no test can catch is dropping `NOT NULL` from `name`: a column of the primary key is `NOT NULL` anyway.

## Consequences

- **A company deleted in `zgm-iam` leaves its rows here.** What happens to them is decided later.
- **A value of the wrong kind, written by SQL by hand, shows when a module reads it.**
- **Any text is a name to the database.** Only the names a module gives through `add_default` will be accepted by the module's functions.
