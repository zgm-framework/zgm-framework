# ADR-0014: A loop of the module, started by a required builder step, deletes every two minutes the users whose date of deletion has come, in every tenant, in batches of 100 with a pause of 300 ms, as the cleanup of `zgm-limits`, and then the tenants past their date, one a statement, their users with them through the cascade

> **Status:** accepted
> **Date:** 2026-11-28
> **Applies to:** `zgm-iam` (`src/jobs.rs`, `Job`, `JobError`, `Iam::builder`, `repository::delete_users_past_their_date`, `repository::delete_tenants_past_their_date`, the indexes `users_pending_deletion_idx` and `tenants_pending_deletion_idx`, `tests/deleting_users.rs`, `tests/deleting_tenants.rs`)

## Context

- **A deleted user waits in `pending_deletion` until `deletes_at`,** and is gone after it ([ADR-0012](0012-delete-user.md)); until then `set_state` can bring it back ([ADR-0013](0013-the-functions-of-one-user-and-set-state.md)). `deletes_at` is set by the clock of the database.
- **The repository has one pattern for periodic jobs:** the module runs them itself, in a loop started by a required step of its builder ([ADR-0015 of `zgm-limits`](../../../zgm-limits/docs/adr/0015-cleanup-in-a-job-loop.md)). The cleanup of `zgm-limits` passes every two minutes and deletes in batches of 100 with a pause of 300 ms between full batches; there a batch of 1,000 scattered rows held them for 1–2 seconds, up to 8.8 under load, and without the pause a backlog of 400,000 rows slowed the other queries down.
- **An application may delete many users at once,** e.g. 100,000 when a tenant leaves, and their dates then come at the same time.
- **A deleted tenant waits for its own date, and its users go with it:** `users.tenant_id` is a foreign key `ON DELETE CASCADE` ([ADR-0007](0007-the-users-table-and-the-states.md), [ADR-0015](0015-the-state-of-a-tenant.md)). While the tenant waits, every function refuses it, so no request touches its users.

## Options considered

**How often the loop passes,** e.g. Anna's date comes on Tuesday at 10:00:

| Option | Pros | Cons |
|---|---|---|
| **A. Every 2 minutes, as in `zgm-limits`** | Anna is gone by 10:02 | a pass every 2 minutes; one with nothing to delete is a short query on an index |
| **B. Every hour** | fewer passes | Anna may stay until 11:00 |

**How the users of a tenant past its date go:**

| Option | Pros | Cons |
|---|---|---|
| **A. With the tenant, through the cascade, one tenant a statement** | one statement; measured fast; nobody waits for the rows of a tenant that waits for deletion | a very large tenant is one long statement |
| **B. In batches of 100 first, then the tenant** | no statement holds more than 100 users | a second job and its query, for a wait nobody has |

**The users of a tenant that is not active,** e.g. the tenant Kowalski was suspended on Monday and Anna's date comes on Tuesday:

| Option | Pros | Cons |
|---|---|---|
| **A. Deleted at their date, as all the others** | the date is a promise kept; the loop is a job of the module for all the tenants, like the cleanup of `zgm-limits` | none found |
| **B. Kept until the tenant is active again** | a suspended tenant changes nothing | the users of a tenant that stays suspended are never deleted |

## Decision

**How often: A. The users of a tenant past its date: A. A tenant that is not active: A** (decided by the maintainer on 2026-11-28: "A", and "well rather yes, A"; on the batches: "take a look in limits at that deletion, because tricks were done there so that it can cope with heavy load, as if somebody suddenly deleted 100k users, so as not to block the database for long"; and on the users of a tenant, B having been proposed: "but why first their users in batches, when the users have a cascade, so the database does it itself", and, shown the measurement, "ok").

```rust
let iam = Iam::builder()
    .pool(pool)
    .cipher(cipher)
    .settings(IamSettings(settings))
    .run_jobs_in_background(|error: JobError| eprintln!("{}: {}", error.job, error.error))
    .build()
    .await?;
```

- **`.run_jobs_in_background(on_error)` is a required builder step.** Without it the code does not compile. `build()` starts the loop with `tokio::spawn`, after the defaults of the settings are written; a `build()` that fails starts nothing.
- **The loop makes its first pass at once, and then one every 2 minutes.**
- **A pass deletes batches of at most 100 users in `pending_deletion` whose `deletes_at` has come, by the clock of the database, in every tenant, whatever its state.** After a full batch it waits 300 ms and takes the next one; a batch that is not full ends the pass. There is no limit of users per pass.
- **2 minutes, 100 users and 300 ms are constants of the module,** not builder settings.
- **After the users, the same pass deletes the tenants in `pending_deletion` whose `deletes_at` has come, one tenant a statement,** with a pause of 300 ms after each, until there is none; the database deletes the users of each with it. The same `DELETE … WHERE ctid = ANY(ARRAY(SELECT ctid … FOR UPDATE SKIP LOCKED))` on the tenants, along the partial index `tenants_pending_deletion_idx`.
- **One statement deletes a batch of users:** `DELETE … WHERE ctid = ANY(ARRAY(SELECT ctid … WHERE state = 'pending_deletion' AND deletes_at <= statement_timestamp() ORDER BY deletes_at LIMIT 100 FOR UPDATE SKIP LOCKED))`, as in `zgm-limits`. `SKIP LOCKED` lets several copies of the application delete at once without waiting for each other.
- **A partial index, `users_pending_deletion_idx` on `deletes_at` where the state is `pending_deletion`,** holds only the users who wait.
- **No margin after the date:** the date is written and compared by the same clock, that of the database, so no difference between the clocks of servers can bring a deletion forward.
- **An error goes to `on_error(JobError { job, error })`** and does not stop the loop; a panic in `on_error` does not either. A job that fails does not keep the other from running in the same pass. `Job` has two jobs, `DeleteUsersPastTheirDate` and `DeleteTenantsPastTheirDate`, printed as `delete_users_past_their_date` and `delete_tenants_past_their_date`. The module logs nothing itself.
- **The loop ends with the last clone of `Iam`,** whose clones share a handle that aborts the task when it is dropped; the loop's own copy does not hold it. A `TenantScope` or a `UserScope` holds a clone too.
- **The two jobs are the queries of the module without a tenant:** jobs of the module itself, that read and give back nothing about any tenant.

## Why

- **The same loop as `zgm-limits`,** whose numbers were measured there under load: a batch of 100 holds its rows for a fraction of a second, so a change of a user that waits for a batch waits that long, and the pause keeps a backlog of 100,000 users to about 300 a second, some six minutes, without keeping the database busy.
- **`FOR UPDATE SKIP LOCKED` in the `SELECT`:** a user that `set_state` is bringing back is either locked, and skipped, or already back, and no longer in `pending_deletion`; it is not deleted.
- **The `ORDER BY` and the partial index:** the statement goes along the index under every plan. Measured on 2026-11-28 on PostgreSQL 18 in Docker, on 1,000,000 users of 1,000 tenants, 20,000 of them past their date, with the plan forced to be generic: a batch of 100 read 408 buffers and took 0.24 ms; a pass with nothing to delete read 2 buffers and took 0.03 ms.
- **A tenant through the cascade:** PostgreSQL only marks the rows it deletes and leaves the indexes to `VACUUM`. Measured on 2026-11-28 on PostgreSQL 18 in Docker, among 1,000 other tenants of 500 users each: a tenant of 50,000 users was deleted in 53 ms, one of 500,000 in 187 ms, where a batch of 100 users took 3.6 ms. The users of a tenant of a real database lie scattered over more pages, so it may take several times longer, and still nobody waits for them. One tenant a statement, so that many large tenants with the same date do not go in one.
- **The users of a tenant that is not active are deleted too:** the date was given to the user, and a suspended tenant would otherwise keep users for ever.
- **Checked by tests** in `tests/deleting_users.rs`, on a database, with the module built again to get a pass at once: a user past its date is gone, and one waiting with a date to come, an active one and a suspended one stay; a user brought back by `set_state` before the pass stays; the users past their date of a suspended tenant and of one waiting for deletion are gone; one pass deletes 250 users, more than two batches; on a closed pool, with the clock of `tokio` paused, every one of four passes goes to `on_error` as `DeleteUsersPastTheirDate` with a database error, though `on_error` panics. 4 of the 5 went red on a pass that deletes nothing; the one left checks that a user brought back stays. In `tests/deleting_tenants.rs`: a tenant past its date is gone with its users, and one waiting with a date to come, a suspended one and an active one stay with theirs; a tenant brought back by `set_state` before the pass stays; one pass deletes four tenants past their date, and their users. On the closed pool each pass reports both jobs. 2 of the 3 went red, and the test of the errors, on a job of the tenants that did nothing; the one left checks that a tenant brought back stays.

## Consequences

- **`build()` called outside a `tokio` runtime panics.**
- **A user is gone at most about two minutes after its date,** later when a backlog of more than about 36,000 users in two minutes is being worked through.
- **Every test that builds `Iam` starts the loop,** as an application does; on its own database it finds nothing to delete unless the test put a date in the past.
- **Nothing tells the module of sessions that a user is gone;** how a deletion in this module ends the sessions of the user is open.
