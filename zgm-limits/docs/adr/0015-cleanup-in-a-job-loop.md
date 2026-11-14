# ADR-0015: Counters that have been dead for an hour are deleted by the module's loop, started by a required builder step

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`src/jobs.rs`, `Limits::builder`, `repository::delete_expired`)

## Context

- **A counter row past its `expires_at` means nothing** ([ADR-0005](0005-counters-table-and-key.md)). A key that comes back uses its row again, but a key that never comes back, e.g. an attacker's IP address, would leave its row forever.
- **The repository has one pattern for periodic jobs** (the same one in every crate by this author; first written down as ADR-0094 of `zgm-credentials`): the application calls no jobs, and the module runs them itself, in one loop.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. The module's loop, started by a required builder step** | the cleanup cannot be forgotten; as in the rest of the repository | `build()` must be called inside a `tokio` runtime |
| **B. A method the application calls itself, e.g. from cron** | no background task | an application that forgets has a table that grows without end |
| **C. Cleaning up along with attempts** | no loop | extra work on the hottest path; keys that nobody touches stay anyway |

**How long to keep a row after `expires_at`:**

| Option | Pros | Cons |
|---|---|---|
| **A. Delete at once** | the smallest table | a server whose clock runs fast deletes live blocks for everybody: with a 5-minute difference, a 15-minute block ends after 10 |
| **B. Delete only after an hour** | a difference between the servers' clocks does not shorten blocks; the list of blocked keys shows recently expired ones | rows stay an hour longer |

## Decision

**The loop: A. Keeping: B** (decided by the maintainer on 2026-11-10 and 2026-11-11: "let the cleanup delete the ones dead for an hour").

- **`.run_jobs_in_background(on_error)` is a required builder step.** Without it the code does not compile. `build()` starts the loop with `tokio::spawn`.
- **The loop makes its first pass at once, and then one every 2 minutes.**
- **A pass deletes batches of 100 rows whose `expires_at` passed at least an hour ago.** After a full batch it waits 300 ms and takes the next one; a batch that is not full ends the pass.
- **2 minutes, 100 rows, 300 ms and an hour are constants of the module,** not builder settings.
- **An error goes to `on_error(JobError { job, error })`** and does not stop the loop. The module logs nothing itself.
- **A panic in `on_error` does not stop the loop either.** The loop catches it and makes the next pass at the usual time. Without this, one panic in the application's function would turn the cleanup off until a restart, and nothing would say so; a code review reproduced it.
- **The loop ends with the last clone of `Limits`.** The clones share a handle that aborts the task when it is dropped, and the loop's own copy of the module does not hold that handle.
- **The query takes rows in the order of `expires_at`, the oldest first.** The `ORDER BY` is there for the plan: with it the database goes along the `expires_at` index under every kind of plan. Without it the generic plan read the whole table: on 1.5 million rows, a pass with nothing to delete took 1107 ms instead of 0.06 ms.
- **The query takes rows with `FOR UPDATE SKIP LOCKED`,** so several copies of the application clean up at once and do not wait for each other.
- **The cleanup is the only query of the module without a tenant** (decided by the maintainer on 2026-11-11: "the cleanup must stay as it is, it cleans up for all tenants"). It is a job of the module itself, not an action on behalf of a tenant: it looks only at `expires_at`, reads and returns nothing about any tenant, and the module knows no list of tenants it could go through.
- **The loop reads the module's clock,** the same one as the attempts ([ADR-0011](0011-clock-from-the-builder.md)).

## Why

- **The cleanup changes nothing in the counting:** `hit` opens a new window on a dead row. It is only about the size of the table.
- **The history of abuse does not belong in the counters table.** Even without the cleanup, a new window overwrites the old one in the same row, so the table shows only the current state. The history goes to the application as events ([ADR-0021](0021-events-instead-of-logs.md)).
- **Checked with tests,** also by breaking the code on purpose: a cleanup that deletes open windows, and a pass that ends after one batch, are both detected.

## Consequences

- **`include_expired` on the list of blocked keys shows the blocks that ended in the last hour,** sometimes a few minutes longer ([ADR-0012](0012-list-of-blocked-keys.md)).
- **The 300 ms pause between full batches** (decided by the maintainer on 2026-11-11) is there so that a large backlog is not deleted in one go. A performance review measured that when 400,000 rows were deleted without a pause, the attempts that pass slowed down in five runs out of five, in the worst one from about 6000 to 2200 per second. With the pause, the database is busy with the cleanup for a small fraction of the time. Our own measurement of the effect of the pause was inconclusive: over five rounds, the variant with no cleanup at all ranged from 140 to 4747 attempts per second, more than the differences between the variants. It confirmed only the rate: in 5 seconds, 14,000–18,000 rows disappeared with the pause, and 112,000–527,000 without it.
- **A batch of 100, not 1000** (decided by the maintainer on 2026-11-11). A batch holds its rows until it is done, and an attempt on a key that comes back just then waits for it. A review under a flood measured that a batch of 1000 dead rows scattered over the table takes 1–2.3 seconds, under load up to 8.8 seconds, and during that time 31 of 32 connections were waiting. A hundred rows is a ten times shorter wait and ten times fewer keys that can run into it.
- **The cleanup deletes at most about 300 rows a second and does not have to keep up at every moment.** When dead rows arrive faster, the table grows for a while, and the cleanup catches up when the traffic falls, e.g. at night. A dead row takes up room and breaks nothing.
- **With a long-open transaction in the database the cleanup slows down, and we do not fix that** (decided by the maintainer on 2026-11-11). As long as such a transaction lasts, PostgreSQL does not remove the index entries of deleted rows, and every batch starts searching from the beginning of the index and goes through them. In a measurement, on an artificially produced state with 105,000 deleted rows, the rate fell from about 300 to 80–170 rows a second; it had no effect on the time of requests. Rejected: a pass that remembers how far it got, because that is extra state in the loop for a case that users do not notice. The cause is removed by the database setting from [ADR-0023](0023-the-database-bounds-waiting-for-a-row.md).
- **There is no hard limit of rows per pass.** With a limit, the cleanup would stop keeping up exactly when dead keys arrive fastest, e.g. during an attack from thousands of addresses.
- **The hour is a margin for the difference between clocks,** not a history of abuse. A code review showed a block shortened by a clock running 5 minutes fast; an hour is far more than the clocks of servers with working time synchronization differ by.
- **`build()` called outside a `tokio` runtime panics.**
- **The tests start the loop the way an application does.** Every test has its own database, and the loop deletes only rows that mean nothing anyway. The cleanup test moves the test clock and builds the module once more to get a pass at a chosen moment.
- **The `Attempts` object holds a clone of `Limits`,** so it too keeps the loop running for as long as it exists.
- **There is a test for an `on_error` that panics:** on a closed pool, with the `tokio` clock paused, the loop calls it in four consecutive passes. Without catching the panic this test does not pass.
- **A caught panic disappears without a trace,** because there is nobody left to report it to. In an application compiled with `panic = "abort"` a panic cannot be caught, and it ends the whole process.
- **The module has no test that the loop ends with the last clone.** A reviewer checked it with a separate test on both kinds of the `tokio` runtime.
- **After a change of the secret key, the old rows disappear the same way,** once their window is over ([ADR-0008](0008-changing-the-secret-key.md)).
