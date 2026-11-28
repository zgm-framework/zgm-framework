# Architecture decision records of `zgm-iam`

Every choice of a concrete solution in this crate has a record here: the context, the options considered with their pros and cons, the decision, and why. A record describes the decision in force today; when a decision changes, its record is rewritten, so there is no trail of superseded records to read through.

A few things that help when reading them:

- **"The maintainer"** is the author of the crate, who makes the decisions.
- **`zgm-limits`** is another crate of this repository. This crate follows its patterns and does not depend on it.
- **Measurements** were taken on one laptop with a local PostgreSQL 18, so absolute numbers will differ elsewhere.

| No. | Decision | Status | Date |
|---|---|---|---|
| [0001](0001-nothing-says-whose-uuid-it-is.md) | The module's tables hold UUIDs in the open and nothing that says whose UUID it is | accepted | 2026-11-15 |
| [0002](0002-the-tenants-table.md) | The module keeps the tenants; a tenant is a UUID v7, an encrypted name and two dates | accepted | 2026-11-15 |
| [0003](0003-the-module-runs-its-migrations.md) | The `migrate` function creates the module's tables, and the migration history lives in the module's schema | accepted | 2026-11-15 |
| [0004](0004-the-cipher-from-the-application.md) | What the module keeps readable nowhere is encrypted by a `Cipher` from the application, required in the builder | accepted | 2026-11-20 |
| [0005](0005-iam-with-a-bon-builder.md) | The entry point of the module is the `Iam` struct, built with a `bon` builder; `create_tenant` refuses an empty name and gives back the whole tenant | accepted | 2026-11-20 |
| [0006](0006-dates-through-jiff.md) | The module's dates are `jiff::Timestamp`, read from the database through `jiff-sqlx` | accepted | 2026-11-20 |
| [0007](0007-the-users-table-and-the-states.md) | A user is a UUID v7 under a tenant, with a state and no name; a tenant and a user are `active`, `suspended` or `pending_deletion`, kept as text | accepted | 2026-11-21 |
| [0008](0008-the-functions-of-one-tenant-and-create-user.md) | The functions of one tenant hang from `Iam::tenant_id`; `create_user` writes a user only under an active tenant | accepted | 2026-11-21 |
| [0009](0009-get-user.md) | `user_id(id).get()` gives back a user of an active tenant in any state of the user, or `None`; a tenant that is not active refuses reading too | accepted | 2026-11-22 |
| [0010](0010-list-users.md) | `list_users` gives the users of an active tenant in pages of at most 100, the newest or the oldest first, of every state or of one | accepted | 2026-11-22 |
| [0011](0011-tenant-settings-from-the-application.md) | The settings of tenants come from the application through a `TenantSettings` trait, required in the builder; `build()` gives the settings a user deletion window of 30 days, from a day to a year, and waits for it to be written | accepted | 2026-11-28 |
| [0012](0012-delete-user.md) | `user_id(id).delete()` puts an active or a suspended user in `pending_deletion` with a date the window of its tenant away; a user who waits already stays as it was, and no such user is `None` | accepted | 2026-11-24 |
| [0013](0013-the-functions-of-one-user-and-set-state.md) | The functions of one user hang from `user_id`, which reads nothing; `set_state` puts a user in `active` or `suspended` from any state, also after its date of deletion, and is the one way to block, unblock and cancel a deletion | accepted | 2026-11-28 |
| [0014](0014-the-loop-that-deletes-users.md) | A loop of the module, started by a required builder step, deletes every two minutes the users whose date of deletion has come, in every tenant, in batches of 100 with a pause of 300 ms, as the cleanup of `zgm-limits`, and then the tenants past their date, one a statement, their users with them through the cascade | accepted | 2026-11-28 |
| [0015](0015-the-state-of-a-tenant.md) | `tenant_id(id).set_state(state)` and `.delete()` change a tenant in any of its states, as for a user; a tenant waits for deletion 90 days, a setting from 7 to 365 days; the tenant comes back with its name decrypted in the transaction of the change | accepted | 2026-11-28 |
