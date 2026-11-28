# ADR-0013: The functions of one user hang from `user_id`, which reads nothing; `set_state` puts a user in `active` or `suspended` from any state, also after its date of deletion, and is the one way to block, unblock and cancel a deletion

> **Status:** accepted
> **Date:** 2026-11-28
> **Applies to:** `zgm-iam` (`TenantScope::user_id`, `UserScope`, `UserScope::set_state`, `IamError::NotSettable`, `src/repository.rs`, `tests/set_state.rs`)

## Context

- **A user is `active`, `suspended` or `pending_deletion`** ([ADR-0007](0007-the-users-table-and-the-states.md)). `delete` puts an active or a suspended user in `pending_deletion` with a date of deletion ([ADR-0012](0012-delete-user.md)), and the loop of the module deletes the users past their date ([ADR-0014](0014-the-loop-that-deletes-users.md)).
- **Deleting a blocked user must not unblock it** ([ADR-0012](0012-delete-user.md)). `pending_deletion` replaces the state the user had, so after a cancellation nothing says whether it was blocked.
- **The functions of a tenant hang from `iam.tenant_id(id)`,** which reads nothing ([ADR-0008](0008-the-functions-of-one-tenant-and-create-user.md)). Reading one user, deleting one and setting its state need the id of the user as well.

## Options considered

**Where a cancelled user returns,** e.g. Anna was blocked, was deleted, and the deletion is cancelled:

| Option | Pros | Cons |
|---|---|---|
| **A. Always to `active`** | nothing to remember | whoever may delete but not unblock unblocks Anna by deleting her and cancelling |
| **B. To the state before the deletion, kept in a column of its own** | Anna comes back blocked, as Microsoft restores a user as it was | a column filled only while a user waits for deletion |
| **C. Always to `suspended`** | nobody comes back unblocked by mistake | an active user comes back blocked, and somebody unblocks it again |
| **D. No function to cancel: one function sets the state, `active` or `suspended`, from any state** | one function for blocking, unblocking and cancelling; who may set the state may unblock anyway, so deleting and cancelling unblocks nobody who could not be unblocked; no column | whoever cancels chooses the state, rather than finding the one before |

**Where the functions of one user hang:**

| Option | Pros | Cons |
|---|---|---|
| **A. On the tenant, with the id of the user: `get_user(id)`, `delete_user(id)`, `set_user_state(id, state)`** | one call fewer on each line | every function of one user, e.g. adding it to a group later, is one more function of the tenant |
| **B. On `in_tenant.user_id(id)`, which reads nothing, as `tenant_id` does: `.get()`, `.delete()`, `.set_state(state)`** | the functions of one user in one place, the way the functions of one tenant are | one more struct, `UserScope` |
| **C. On the user `get_user(id)` gives back: `get_user(id).set_state(state)`** | reads as the maintainer wrote it | a read and a write, two queries, and between them somebody else may change the user |

**A user whose date of deletion has passed,** before the loop has deleted it: the state set as for any user, or an error.

## Decision

**Where a cancelled user returns: D. Where the functions hang: B. After the date of deletion: as for any user** (decided by the maintainer on 2026-11-28: on cancelling, "let there be no method remove_pending_deletion or something like that, only one to change the state, and the state can simply be changed to whatever one wants", and "well I guess that is the simplest, right"; on the date, "these methods on their own have no logic, so it can be done in one simple method?"; on where they hang, the maintainer asked "or maybe `selected_tenant.get_user(anna_id).set_state(State::Active)`", and, shown that it reads first, "well then B, maybe like that?").

```rust
let in_kowalski = iam.tenant_id(kowalski.id);
in_kowalski.user_id(anna.id).set_state(State::Suspended).await?; // Some(anna): blocked
in_kowalski.user_id(anna.id).delete().await?;                    // Some(anna): waits for deletion
in_kowalski.user_id(anna.id).set_state(State::Active).await?;    // Some(anna): back, no date
```

- **`in_tenant.user_id(user_id) -> UserScope`** reads nothing. `UserScope` has `get()` ([ADR-0009](0009-get-user.md)), `delete()` ([ADR-0012](0012-delete-user.md)) and `set_state(state)`, each `Result<Option<User>, IamError>`, `None` when the tenant has no such user, a user of another tenant too. `create_user` and `list_users` stay functions of the tenant.
- **`set_state(State::Active | State::Suspended)`** puts the user in that state from any state, with no date of deletion. A user who waits for deletion comes back also after its date, as long as the loop has not deleted its row.
- **The state given again changes nothing,** and not even `updated_at` moves; the user is given back as it is.
- **`State::PendingDeletion` is `IamError::NotSettable`,** before the database is asked: a user waits for deletion only through `delete`, which gives it its date.
- **The tenant is checked as for `delete`:** in one transaction, its state is locked (`FOR SHARE`), and a tenant that does not exist is `NoSuchTenant`, one that is not active `TenantNotActive(state)`, also for an id of no user.
- **One `UPDATE`:** `SET state = $3, deletes_at = NULL WHERE tenant_id = $1 AND id = $2 AND state IN ('active', 'suspended', 'pending_deletion') AND state <> $3 RETURNING …`. When it changes nothing, a `SELECT` of the user of the tenant says why: none, the state given already, or a word that is no state, which is `UnknownState` and stays in the row. That `SELECT` is shared with `delete`.

## Why

- **One function rather than three:** blocking, unblocking and cancelling only put a word in the row, and the maintainer wants the simplest. Who may set the state is a matter of permissions, for all three alike, so deleting and cancelling opens no way round a block.
- **`user_id` as `tenant_id`:** one way to reach what belongs to something, with no query until a function is called.
- **After the date:** a check of the clock would add a rule to a function that has none; the user is gone when the loop deletes the row.
- **`PendingDeletion` refused:** a user waiting for deletion without a date would never be deleted.
- **A word the module does not know is left in the row,** as in `delete`: only SQL by hand writes one, and the module does not overwrite what it cannot read.
- **Checked by tests** in `tests/set_state.rs`, on a database: an active user is suspended and active again, each change moving `updated_at`, and what comes back is the row; a user waiting for deletion comes back active or suspended without a date; one whose date has passed comes back too; the state given again changes nothing, not even `updated_at`; `PendingDeletion` is `NotSettable` and nothing changes; an id of no user and a user of another tenant are `None` and nothing changes; a tenant that does not exist is `NoSuchTenant`; a suspended tenant and one waiting for deletion refuse, also for an id of no user; a word that is no state is `UnknownState`, and the row keeps it. 8 of the 10 went red on a `set_state` that changed nothing; the two that passed check `None`.

## Consequences

- **Cancelling a deletion means choosing the state,** `active` or `suspended`; the application shows what the user was, if it keeps it.
- **A user can be unblocked from `pending_deletion` straight to `active`:** whoever may set the state may do that, as with any user.
- **The functions of one user to come,** e.g. adding it to a group, hang from `user_id` too.
