# Architecture decision records of `zgm-settings`

Every choice of a concrete solution in this crate has a record here: the context, the options considered with their pros and cons, the decision, and why. A record describes the decision in force today; when a decision changes, its record is rewritten, so there is no trail of superseded records to read through.

A few things that help when reading them:

- **"The maintainer"** is the author of the crate, who makes the decisions.
- **`zgm-limits`** is another crate of this repository, and **`zgm-iam`** a module of it still being built, which will read its settings through this one. This crate follows their patterns and depends on neither.
- **Measurements** were taken on one laptop with a local PostgreSQL 18, so absolute numbers will differ elsewhere.

| No. | Decision | Status | Date |
|---|---|---|---|
| [0001](0001-settings-in-a-module-of-their-own.md) | The settings of tenants are a module of their own, which other modules reach through a trait; `zgm-limits` keeps its own table | accepted | 2026-11-23 |
| [0002](0002-defaults-built-into-the-modules.md) | The defaults and their bounds are built into the modules and written to the database once, when a module first starts; later only a function of the settings module changes them | accepted | 2026-11-24 |
| [0003](0003-the-tenant-settings-table.md) | Two tables: `default_settings`, one row per setting with its kind, its value as `jsonb` and its bounds as `bigint`; `tenant_settings`, one row per tenant and setting under the key `(tenant_id, name)`, its name a foreign key to the default; the database checks no kind and no bound | accepted | 2026-11-24 |
| [0004](0004-the-module-runs-its-migrations.md) | The `migrate` function creates the module's tables, and the migration history lives in the module's schema | accepted | 2026-11-23 |
| [0005](0005-settings-and-its-defaults.md) | `Settings` is built with a `bon` builder; a module gives each default with its bounds as a `DefaultSetting` to `add_default`, which writes the row once, checks nothing, and keeps in memory what the database has | accepted | 2026-11-26 |
| [0006](0006-get-a-setting-of-a-tenant.md) | The functions of one tenant hang from `Settings::tenant_id`; `get` reads the default and the tenant's value from the database every time, in one statement, and gives the tenant's value as it is, also outside bounds lowered after it was set | accepted | 2026-11-26 |
| [0007](0007-set-a-setting-of-a-tenant.md) | `set` reads the kind and the bounds of the default from the database, refuses a value outside the bounds in force, also the value the tenant has, and writes or changes the row of the tenant in one statement, saying whether anything changed | accepted | 2026-11-26 |
| [0008](0008-restore-the-default.md) | `restore_default` deletes the row of a tenant without reading it, in one statement that also finds the default in the database, and says whether there was a row; a name without a default is refused | accepted | 2026-11-26 |
| [0009](0009-list-the-settings-of-a-tenant.md) | `list` gives every setting that has a default, in the order of the names, with the value in the tenant, the default and whether the tenant changed it; no pages | accepted | 2026-11-23 |
