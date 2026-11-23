# Architecture decision records of `zgm-settings`

Every choice of a concrete solution in this crate has a record here: the context, the options considered with their pros and cons, the decision, and why. A record describes the decision in force today; when a decision changes, its record is rewritten, so there is no trail of superseded records to read through.

A few things that help when reading them:

- **"The maintainer"** is the author of the crate, who makes the decisions.
- **`zgm-limits`** is another crate of this repository, and **`zgm-iam`** a module of it still being built, which will read its settings through this one. This crate follows their patterns and depends on neither.
- **Measurements** were taken on one laptop with a local PostgreSQL 18, so absolute numbers will differ elsewhere.

| No. | Decision | Status | Date |
|---|---|---|---|
| [0001](0001-settings-in-a-module-of-their-own.md) | The settings of tenants are a module of their own, which other modules reach through a trait; `zgm-limits` keeps its own table | accepted | 2026-11-23 |
| [0002](0002-defaults-built-into-the-modules.md) | The defaults are built into the modules, given to the settings module when they are built and kept in memory; the database holds only what tenants changed | accepted | 2026-11-23 |
| [0003](0003-the-tenant-settings-table.md) | `zgm_settings.tenant_settings` has one row per tenant and setting: the name with the prefix of its module, the value as `jsonb`, the key `(tenant_id, name)` and no foreign key | accepted | 2026-11-23 |
| [0004](0004-the-module-runs-its-migrations.md) | The `migrate` function creates the module's table, and the migration history lives in the module's schema | accepted | 2026-11-23 |
| [0005](0005-settings-and-its-defaults.md) | `Settings` is built with a `bon` builder; the modules give it defaults of the four kinds of `Value`, which it keeps in memory, shared by its clones; a default given again replaces the one before | accepted | 2026-11-23 |
| [0006](0006-get-a-setting-of-a-tenant.md) | The functions of one tenant hang from `Settings::tenant_id`; `get` gives the tenant's value or the default, read as the kind of the default from plain JSON | accepted | 2026-11-23 |
| [0007](0007-set-a-setting-of-a-tenant.md) | `set` writes or changes the row of a tenant in one statement and says whether anything changed; it checks the kind and whole seconds, and no range | accepted | 2026-11-23 |
| [0008](0008-restore-the-default.md) | `restore_default` deletes the row of a tenant without reading it and says whether there was one; a name without a default is refused | accepted | 2026-11-23 |
| [0009](0009-list-the-settings-of-a-tenant.md) | `list` gives every setting that has a default, in the order of the names, with the value in the tenant, the default and whether the tenant changed it; no pages | accepted | 2026-11-23 |
