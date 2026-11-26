# ADR-0003: Two tables: `default_settings`, one row per setting with its kind, its value as `jsonb` and its bounds as `bigint`; `tenant_settings`, one row per tenant and setting under the key `(tenant_id, name)`; the database checks no kind and no bound

> **Status:** accepted
> **Date:** 2026-11-24
> **Applies to:** `zgm-settings` (`migrations/20261123105422_create_settings_tables.sql`, `tests/default_settings_table.rs`, `tests/tenant_settings_table.rs`)

## Context

- **The defaults and their bounds lie in the database, one row per setting, and a tenant's own value in a row of its own** ([ADR-0002](0002-defaults-built-into-the-modules.md)). E.g. the user deletion window is 30 days for every tenant, at least a day and at most a year, and the tenant Nowak keeps 90 days.
- **Settings come from several modules,** and each names its settings with an enum.
- **A setting is of one of four kinds:** a duration in whole seconds, a whole number, yes or no, or a text. Only a duration and a number have bounds.
- **Only the module's code writes both tables,** and a default changes only through a function of the module ([ADR-0002](0002-defaults-built-into-the-modules.md)).
- **The tenants live in `zgm-iam`,** and the modules do not depend on each other.
- **`zgm-limits` has a table of the same purpose as `tenant_settings`,** `tenant_rules`: the key `(tenant_id, limit_name)`, the rule in `jsonb`, two dates and a trigger for `updated_at`.

## Options considered

**The name of a setting:**

| Option | Pros | Cons |
|---|---|---|
| **A. The prefix of the module in one column:** `iam.deletion_window` | one column; two modules cannot give the same name; the row says whose setting it is | a longer text in every row |
| **B. A column for the module:** (`iam`, `deletion_window`) | the module can be queried apart | a key of three columns in `tenant_settings` |
| **C. No prefix; the same name given twice stops the program at startup** | the shortest names | the names must differ across all modules at once |

**The value, in both tables:**

| Option | Pros | Cons |
|---|---|---|
| **A. `jsonb`** | one column for every kind of setting; readable in the database; the same as `tenant_rules` | the database checks only that it is JSON, not that it is a number of seconds |
| **B. `text`** | no JSON | the database checks nothing |
| **C. `bigint`** | the database checks that it is a number | numbers only; a flag or a text would need other columns |

**The kind of a default:**

| Option | Pros | Cons |
|---|---|---|
| **A. A column `kind`:** `duration`, `number`, `flag` or `text` | the row says what its value is: a number of seconds and a plain number are the same JSON | one more column |
| **B. Only in the module's code** | one column less | the row alone does not say whether `2592000` is seconds or a count |

**The bounds:**

| Option | Pros | Cons |
|---|---|---|
| **A. Two `bigint` columns, `min` and `max`, `NULL` for no bound** | the database itself refuses a word or `true` in a bound; a whole number holds both whole seconds and a number | a bound of another kind would need another column |
| **B. `jsonb`, as the value** | the same type as the value | the database takes anything as a bound |

**Rules in the database for `default_settings`** (a kind it knows, the least not above the most, no bounds on a flag or a text):

| Option | Pros | Cons |
|---|---|---|
| **A. None; the module's code checks them** | each rule lives in one place | SQL by hand can write a row the module will not accept |
| **B. `CHECK` constraints** | SQL by hand cannot break a rule | each rule in two places, the code and the database, which drift apart |

**The names of the tables:** `tenant_settings`, `settings`, `overrides` (what it holds: only changes against the defaults), `tenant_overrides`; and for the defaults, `default_settings`.

## Decision

**Name: A. Value: A. Kind: A. Bounds: A. Rules: A. Tables: `default_settings` and `tenant_settings`** (decided by the maintainer: on 2026-11-23 "well, let it be A", "A", and on the table of tenants "or actually let it be `tenant_settings`", the name from the module's enum through `strum`: "and it has to have strum, which turns that name in the enum into a string"; on 2026-11-24 "ok A" on the kind, "A" on the bounds, "yes" on `default_settings`, and on the rules: "only a function of the module changes it").

```sql
CREATE TABLE zgm_settings.default_settings (
    name        text        NOT NULL,   -- e.g. 'iam.user_deletion_window'
    kind        text        NOT NULL,   -- 'duration', 'number', 'flag' or 'text'
    value       jsonb       NOT NULL,   -- e.g. 2592000
    min         bigint,                 -- e.g. 86400; NULL is no bound
    max         bigint,                 -- e.g. 31536000; NULL is no bound
    created_at  timestamptz NOT NULL DEFAULT statement_timestamp(),
    updated_at  timestamptz NOT NULL DEFAULT statement_timestamp(),
    CONSTRAINT default_settings_pkey PRIMARY KEY (name)
);

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
- **`default_settings` is keyed by the name alone,** and `tenant_settings` by the tenant first and then the name. Neither has an id of its own. The triggers `touch_default_settings` and `touch_tenant_settings` set `updated_at` on every change.
- **The four words of `kind` are the kinds of the Rust enum `Value`.** A duration is kept in whole seconds, in the value and in the bounds.
- **There is no foreign key to the tenants.** `tenant_id` is a plain UUID, as in `zgm-limits`.
- **Going back to the default deletes the row of the tenant.** Neither table has soft deletion.
- **The database checks no name, no kind and no bound,** and no value beyond JSON. The code that writes and reads a setting checks them.

## Why

- **The prefix:** two modules may both have a `DeletionWindow`, e.g. the days a user waits in `zgm-iam` and the days old sessions are kept in a module of sign-in; without a prefix both would be `deletion_window`. Checked on `strum` 0.28.0: with `serialize_all = "snake_case", prefix = "iam."`, `IamSetting::UserDeletionWindow` gives `iam.user_deletion_window`.
- **`jsonb`:** one column holds every kind of setting, and the module that reads a setting is the one that knows what it must be.
- **`kind`:** 30 days and the number 2592000 are the same JSON, so without the column a row could not say which it holds.
- **`bigint` for the bounds:** only a duration in whole seconds and a whole number have bounds, and both are whole numbers of 64 bits. Checked on PostgreSQL 18: a bound written as the text `'1.5'`, `'a day'` or `'true'` is refused. A fraction written as a number, `1.5` without quotes, is rounded to `2`; the module writes whole numbers from Rust, so it never sends one.
- **No rules in the database:** only the module's code writes the tables, so it is the one place the rules live. The database keeps what that code cannot guarantee on its own, one row per name and one per tenant and name, and the dates it sets itself.
- **The keys:** a default is always found by its name, and a row of a tenant by its tenant and its name. The tenant first, so that the same index finds one setting of a tenant and all of its settings.
- **No foreign key to the tenants:** the tenants are in another module, and a key across modules would tie them together.
- **`default_settings` checked by tests,** with SQL by hand: a new row gets the moment of its statement as both dates, also late in a transaction; a change sets `updated_at` to the moment of the change, whatever the statement says; the same name twice is refused even when every other column differs, and the first row stays; two settings are two rows; a missing bound is kept empty; the bounds keep the least and the greatest number of 64 bits; a bound written as a fraction or a word is refused; a row without a name, a kind or a value is refused; `'30 days'` is refused as no JSON; a value of each of the four kinds is kept as written; an unknown kind, a least above a most and bounds on a flag are all accepted. Then the table was broken on purpose, one change at a time (26 changes: the key of the name and the kind, or no key; the value as `text` or `json`, or without `NOT NULL`; the kind without `NOT NULL`; each bound as `numeric`, `integer` or `text`, `min` also as `jsonb`, each `NOT NULL`; the dates from `now()`, `updated_at` without a default; the trigger missing, after the update or with `now()`; the table in `public`; a `CHECK` on the kind, on the order of the bounds, on the bounds of a flag). 25 times a test went red. The one change no test can catch is dropping `NOT NULL` from `name`: a column of the primary key is `NOT NULL` anyway.
- **`tenant_settings` checked by tests,** with SQL by hand: a new row gets the moment of its statement as both dates, also late in a transaction; a change sets `updated_at` to the moment of the change, whatever the statement says; the same setting twice in one tenant is refused and the first value stays; the same setting in two tenants and two settings of one tenant are two rows each; a row without a tenant, a name or a value is refused; `'90 days'` is refused as no JSON; a number, `true`, a text and an object are kept as written; a row can be deleted; the key is `tenant_id`, then `name`. Each of these, and the migration history in the module's own schema, was broken on purpose one at a time (19 changes, among them the key in another order or of one column, the value as `text`, `bigint` or `json`, the dates from `now()`, the trigger with `now()`, after the update or missing, the table in `public`, deletes refused). 18 times a test went red; again the one change left was dropping `NOT NULL` from `name`.

## Consequences

- **A tenant deleted in `zgm-iam` leaves its rows in `tenant_settings`.** What happens to them is decided later.
- **A value of the wrong kind, written by SQL by hand, shows when a module reads it.**
- **Any word is a kind and any number a bound to the database,** and a least above a most is kept. Only the module's functions refuse them.
- **Any text is a name to the database.** Only the names a module gives as defaults are accepted by the module's functions.
