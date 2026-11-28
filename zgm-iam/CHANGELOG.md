# Changelog

## 0.1.0 (2026-11-28)

The first version.

- **Tenants** (`Iam::create_tenant`) with a name the application's `Cipher` encrypts; the table holds only the ciphertext.
- **Users** of one tenant (`Iam::tenant_id`): `create_user`, `list_users` in pages of 100, and for one user (`TenantScope::user_id`) `get`, `set_state` and `delete`.
- **States** (`State`): `active`, `suspended` and `pending_deletion`, for tenants and users; `set_state` blocks, unblocks and cancels a deletion.
- **Deletion after a window:** 30 days for a user and 90 for a tenant, settings of the tenant with bounds, read through the application's `TenantSettings`, e.g. `zgm-settings`.
- **A loop of the module** (`run_jobs_in_background`) that deletes the users and the tenants past their date every two minutes, in batches.
- **Two tables** the module creates itself (`migrate`).
