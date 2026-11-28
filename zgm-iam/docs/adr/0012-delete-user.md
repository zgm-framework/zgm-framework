# ADR-0012: `user_id(id).delete()` puts an active or a suspended user in `pending_deletion` with a date the window of its tenant away; a user who waits already stays as it was, and no such user is `None`

> **Status:** accepted
> **Date:** 2026-11-24
> **Applies to:** `zgm-iam` (`UserScope::delete`, `src/repository.rs`, `src/error.rs`)

## Context

- **A user is deleted after a window:** until its date of deletion the user waits in `pending_deletion`, nothing works on it but cancelling the deletion, and after the date the row is gone ([ADR-0007](0007-the-users-table-and-the-states.md)). The date is kept in `deletes_at`, fixed when the deletion is asked for, so a later change of the window moves nobody's date.
- **The window is a setting of the tenant,** `iam.user_deletion_window`, 30 days unless the tenant set otherwise, read through the application's `TenantSettings` ([ADR-0011](0011-tenant-settings-from-the-application.md)). `zgm-settings` holds it within the bounds this module gives, from a day to a year ([ADR-0011](0011-tenant-settings-from-the-application.md)); this module takes any length it gets, 0 too.
- **`user_id(id).get()` gives `None` for a user the tenant does not have,** and refuses with `NoSuchTenant` or `TenantNotActive` in a tenant that does not exist or is not active ([ADR-0009](0009-get-user.md)). `create_user` locks the state of the tenant (`FOR SHARE`), so a suspension at the same moment is waited for ([ADR-0008](0008-the-functions-of-one-tenant-and-create-user.md)).
- **How others name it:** Microsoft Graph and the Google Admin SDK call it `delete`, and keep a deleted user for 30 and 20 days; AWS IAM's `DeleteUser` deletes at once, and an unknown user is the error `NoSuchEntity`.
- **The dates of the module are `jiff::Timestamp`** ([ADR-0006](0006-dates-through-jiff.md)), which ends with the year 9999. PostgreSQL keeps later dates: checked on 2026-11-24, `statement_timestamp()` plus 9 000 years gives a date in the year 11020, which the module could not read back.

## Options considered

**A user who waits for deletion already:**

| Option | Pros | Cons |
|---|---|---|
| **A. Nothing changes; the user is given back with its date** | a second click, or two people at once, does no harm; the same as `set` in `zgm-settings` with the value it has | none found |
| **B. An error** | the application learns it was done before | an error to handle where nothing went wrong |
| **C. A new date, from now** | the user gets the whole window again | after the tenant shortens its window, the user is gone sooner than promised |

**A suspended user:**

| Option | Pros | Cons |
|---|---|---|
| **A. Waits for deletion like an active one** | blocking and deleting go the same way: the user can do nothing | where the user returns after a cancellation is decided with the cancellation |
| **B. An error: unblock first** | a user waiting for deletion was always active before | to delete the user, the application makes it active for a moment |

**No such user, or a user of another tenant:**

| Option | Pros | Cons |
|---|---|---|
| **A. `None`** | the module says "there is none" one way, as `get` and `HashMap::remove` do | an application that ignores the result does not notice |
| **B. The error `NoSuchUser`** | the way of AWS | two ways to say "there is none" in one module |

**A window that makes no sense, e.g. 0 days:**

| Option | Pros | Cons |
|---|---|---|
| **A. Any length is taken; only a date the module cannot hold is an error** | the mechanism works with any number; 0 deletes at once, as AWS IAM does | a tenant that sets 0 by mistake loses the user at the next run of the loop that deletes; a check belongs where the tenant types the number |
| **B. A bound of the module, e.g. at least a day** | a mistaken 0 deletes nobody at once | the module imposes a number; the error shows when a user is deleted, not when the number is set |
| **C. The built-in 30 days instead** | a mistaken 0 deletes nobody at once | the choice of the tenant is dropped, and nobody is told |

**The name:** `delete` or `schedule_deletion`.

## Decision

**Waiting already: A. Suspended: A. No such user: A. The window: A. The name: `delete`** (decided by the maintainer on 2026-11-24, one by one: "well yes, nothing changes, if it is in `PendingDeletion` already a second click does nothing"; "A"; "A"; "well, A"; "A"; the design as a whole: "ok").

- **`iam.tenant_id(id).user_id(user_id).delete() -> Result<Option<User>, IamError>`;** `user_id` reads nothing, and `set_state` brings a user back ([ADR-0013](0013-the-functions-of-one-user-and-set-state.md)).
- **First it asks `TenantSettings::duration(tenant_id, "iam.user_deletion_window")`.** An error there is `IamError::Settings`, and nothing changes.
- **A window that puts the date past the end of the year 9999 is `IamError::DeletionDateOutOfRange`,** and nothing changes. It is checked against the clock of the application before the transaction; the clock of the database may differ by seconds, which matters only at the very end of the year 9999. Any shorter window, 0 too, is taken.
- **Then, in one transaction, it locks the state of the tenant (`FOR SHARE`)** and refuses as `create_user` does: `NoSuchTenant`, or `TenantNotActive` with the state, even for an id of no user.
- **One `UPDATE` puts an `active` or `suspended` user of the tenant in `pending_deletion`,** with `deletes_at = statement_timestamp() + window`, so the date is the moment of the change, by the clock of the database, the same moment as `updated_at`. The window goes as an interval of whole microseconds (`PgInterval`), the precision of the database; a number times `interval '1 second'` would go through a double and could lose microseconds.
- **When the `UPDATE` changes nothing, the user is read as it is:** a user who waits already comes back with its date and its `updated_at` unmoved; a user the tenant does not have is `None`; a state the module does not know is `IamError::UnknownState`, and the row keeps it.
- **All SQL is in `src/repository.rs`** (`schedule_deletion_of_user`).

## Why

- **Asking twice should not hurt,** and a date once given is a promise: nobody is deleted sooner than they were told.
- **Deleting a blocked user must not unblock it,** not even for a moment.
- **One way to say "there is none"** across the module.
- **The module gives the mechanism, not a value:** whether 0 days is right is the tenant's choice. The bounds this module gives with its default, from a day to a year, are checked by `zgm-settings` when a tenant sets a value ([ADR-0011](0011-tenant-settings-from-the-application.md)); `delete_user` still takes what it is given.
- **The settings are asked before the transaction,** so the lock on the tenant is not held while the application's settings answer.

## Consequences

- **The settings are asked even when the tenant does not exist or the user waits already;** the answer is then not used.
- **0 days gives a date equal to the moment of the change,** and the user is gone at the next pass of the loop that deletes, at most two minutes later ([ADR-0014](0014-the-loop-that-deletes-users.md)); `zgm-settings` lets a tenant set no less than a day.
- **The user's sessions in the module of logins are not ended here;** how a deletion in IAM reaches them is open.
- **A tenant suspended at the same moment is waited for:** the user waits for deletion either before the suspension or not at all.
