# ADR-0015: `tenant_id(id).set_state(state)` and `.delete()` change a tenant in any of its states, as for a user; a tenant waits for deletion 90 days, a setting from 7 to 365 days; the tenant comes back with its name decrypted in the transaction of the change

> **Status:** accepted
> **Date:** 2026-11-28
> **Applies to:** `zgm-iam` (`TenantScope::set_state`, `TenantScope::delete`, `IamSetting::TenantDeletionWindow`, `IamError::UnreadableName`, `Iam::tenant_of`, `src/repository.rs`, `tests/tenant_state.rs`)

## Context

- **A tenant is `active`, `suspended` or `pending_deletion` with a date, as a user is,** and is deleted only after a window, its users with it ([ADR-0002](0002-the-tenants-table.md), [ADR-0007](0007-the-users-table-and-the-states.md)).
- **Nothing works in a tenant that is not active** ([ADR-0008](0008-the-functions-of-one-tenant-and-create-user.md), [ADR-0009](0009-get-user.md)): `create_user` and every function of a user refuse with `TenantNotActive`.
- **A user is changed by `set_state` and `delete`,** with a window that is a setting of the tenant, from a day to a year, 30 days unless it set otherwise ([ADR-0011](0011-tenant-settings-from-the-application.md), [ADR-0012](0012-delete-user.md), [ADR-0013](0013-the-functions-of-one-user-and-set-state.md)).
- **AWS keeps a closed account for 90 days** ([AWS Account Management](https://docs.aws.amazon.com/accounts/latest/reference/manage-acct-closing.html)), which here is closer to a tenant than to a user.
- **The name of a tenant is kept as the bytes the application's cipher made of it** ([ADR-0004](0004-the-cipher-from-the-application.md)); a `Tenant` holds it readable.

## Options considered

**How long a tenant waits for deletion:**

| Option | Pros | Cons |
|---|---|---|
| **A. A constant of the module, 90 days, as AWS** | nothing to set | a program that wants another number cannot have it |
| **B. A setting, as the user deletion window: 90 days, from 7 to 365** | the same mechanism as for a user; a number can be changed in `zgm-settings` | the tenant sets how long it waits for its own deletion |
| **C. The user deletion window of the tenant, 30 days unless it set otherwise** | no new setting | a tenant and a user wait the same, though a tenant takes all its users with it |

The rest of the design followed the functions of a user; its one question of its own was what the functions give back, as a `Tenant` has a name that must be decrypted.

## Decision

**B, 90 days from 7 to 365** (decided by the maintainer on 2026-11-28: "well let it be min 7 max 365, as for the user, and 90 by default"; the design, "ok").

```rust
let in_kowalski = iam.tenant_id(kowalski.id);
in_kowalski.set_state(State::Suspended).await?; // Some(kowalski): blocked
in_kowalski.delete().await?;                    // Some(kowalski): waits 90 days
in_kowalski.set_state(State::Active).await?;    // Some(kowalski): back, no date
```

- **`TenantScope::set_state(state) -> Result<Option<Tenant>, IamError>` and `TenantScope::delete() -> Result<Option<Tenant>, IamError>`;** `None` when there is no such tenant. They work in any state of the tenant, as they are the way out of those states; every other function of the tenant still refuses one that is not active.
- **`set_state` as for a user:** `Active` or `Suspended` from any state, with no date of deletion; the state given again changes nothing, not even `updated_at`; `PendingDeletion` is `NotSettable`; a word that is no state is `UnknownState` and stays in the row.
- **`delete` as for a user:** an active or a suspended tenant waits for deletion until the moment of the change, by the clock of the database, and its window; one that waits already stays as it was, with its date. The window is asked of the settings first; a date past the year 9999 is `DeletionDateOutOfRange`.
- **`IamSetting::TenantDeletionWindow`, `iam.tenant_deletion_window`:** 90 days, at least 7 and at most 365, given by `build()` after the user deletion window.
- **The tenant comes back with its name decrypted, in the transaction of the change:** the `UPDATE … RETURNING` gives the bytes of the name, the cipher decrypts them, and only then is the change committed. A cipher that fails is `IamError::Cipher`, bytes that are not UTF-8 `IamError::UnreadableName`, and in both nothing changes.
- **The users of the tenant stay as they are** while the tenant waits or is blocked; nothing works on them until it is active again.

## Why

- **The same mechanism as for a user:** the maintainer's choice, and one way of changing a state for the whole module.
- **They work in a tenant that is not active,** or a blocked tenant could never be unblocked.
- **The name decrypted before the commit:** the application gets the tenant as it is, and an error never comes with a change already made. The row of the tenant stays locked for the call of the cipher, so a `create_user` in that tenant at the same moment waits that long.
- **Checked by tests** in `tests/tenant_state.rs`, on a database: an active tenant waits 90 days from the moment of the change, with its name, and what comes back is the row; the window the tenant set is the one used, asked once; a suspended tenant waits too; one waiting already stays as it was after its window is shortened; a tenant is suspended, refuses `create_user`, and is active again and takes it; one waiting comes back active or suspended without a date; the same state again changes nothing; `PendingDeletion` is `NotSettable`; a tenant that does not exist is `None` for both; a word that is no state is `UnknownState` for both and stays; a cipher that fails and one that gives bytes that are not UTF-8 change nothing; an error of the settings and a window past the year 9999 change nothing; the users stay while the tenant waits and comes back. 12 of the 14 went red on functions that changed nothing; the two that passed check `None` and the users. The test of `build()` expects both windows.

## Consequences

- **A blocked tenant or one waiting for deletion is changed only by these two functions;** the application decides who may call them.
- **The tenant chooses, in `zgm-settings`, how long it waits for its own deletion,** within 7 to 365 days.
- **A tenant past its date is deleted by the loop of the module,** one tenant a statement, its users with it through the cascade ([ADR-0014](0014-the-loop-that-deletes-users.md)).
