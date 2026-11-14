# ADR-0021: The module does not log; it reports events to the `Audit` given by the application; the event of a change goes in the transaction of that change, and the event of an attempt goes after the fact

> **Status:** accepted
> **Date:** 2026-11-11
> **Applies to:** `zgm-limits` (`src/audit.rs`, `Limits::builder().audit(...)`, `Attempts::hit`, `Attempts::hit_distinct`, `Attempts::reset`, `Limits::unblock`, `override_rule`, `restore_default`, `src/jobs.rs`)

## Context

- **The counters table shows only the current state.** A new window overwrites the old one in the same row, `reset` and `unblock` delete the row, and cleanup deletes rows an hour after their end ([ADR-0015](0015-cleanup-in-a-job-loop.md)). Afterwards there is no sign that somebody was abusing the system (the maintainer, 2026-11-10).
- **The module has no logs and no tracing.**
- **`zgm-audit` writes an event in the transaction of the operation, or separately when the operation ends in a refusal anyway.**
- **A review under a flood of requests showed the hole in reporting after the fact:** a request interrupted between the change and the report leaves the change without an event. With requests interrupted on purpose, 81 of 300 rule changes were saved without `RuleChanged`. Whoever can drop the connection at the right moment unblocks an account or changes a rule without a trace.

## Options considered

**How tightly an event is bound to the change:**

| Option | Pros | Cons |
|---|---|---|
| **A. Everything after the fact, like a log line** | an attempt stays one query; a failure of the receiver breaks nothing | a change made by a person can be left without a trace |
| **B. Everything in a transaction** | the event and the change together or not at all, for attempts too | every attempt and every refusal is 3–4 statements instead of one; measured below |
| **C. Changes in a transaction, attempts after the fact** | operations that change who can get in do not succeed without an event; an attempt stays one query | the trait has two methods; `KeyBlocked` can be lost |

**What to report:**

| Option | Pros | Cons |
|---|---|---|
| **A. Whatever would get a log line** | abuse is visible while it is happening: refusals, blocks, lifted blocks, rule changes, cleanup | under a flood of refusals there are as many events as refusals |
| **B. Only changes of state, without refusals** | few events | it is not visible how many attempts were turned away by a block |
| **C. Every attempt, successful ones too** | the full picture | an event on every ordinary request |

## Decision

**Binding: C. Scope: A** (decided by the maintainer on 2026-11-11, after option B was built and measured).

- **The `Audit` trait has two methods:**
  - `async fn record_in_transaction(&self, transaction: &mut PgConnection, event) -> Result<(), AuditError>` for events of a change;
  - `fn record_after_the_fact(&self, event)` for the rest.
- **The builder requires `.audit(...)`.**
- **Five events:**

| Event | When | How |
|---|---|---|
| `BlockLifted { key, by }` | a block in force was lifted, by `Reset` or `Unblock` | in the transaction |
| `RuleChanged { limit_name, tenant_id, rule }` | a tenant changed a rule or went back to the default (`rule: None`) | in the transaction |
| `AttemptRefused { key, until }` | an attempt was refused | after the fact |
| `KeyBlocked { key, until }` | an attempt passed and filled the window | after the fact |
| `ExpiredDeleted { counters }` | a cleanup pass deleted rows | after the fact |

- **In the transaction:** `reset`, `unblock`, `override_rule` and `restore_default` open a transaction, make the change, call `record_in_transaction` with that transaction, and only then commit it. An audit error, or a panic in the audit, rolls the change back and comes back as `LimitsError::Audit`.
- **After the fact:** `hit` and `hit_distinct` stay one query without a transaction. The module returns the connection to the pool and only then calls `record_after_the_fact`, which returns no error, and a panic in it is caught.
- **Ordinary work reports nothing:** an attempt that passed and did not block; a `reset` that only forgot attempts; the removal of a block that had already ended; the same rule saved a second time; a cleanup pass that found nothing.
- **An event does not contain the hidden part of the key or its hash,** only a `has_hidden_part` flag.
- **Errors still go back to the caller** (`LimitsError`), and errors of the cleanup loop go to `on_error` ([ADR-0015](0015-cleanup-in-a-job-loop.md)). They are not events.

## Why

- **The transaction protects against "the change is there, but there is no trace of it".** A refusal changes nothing, so there is nothing to write it together with. `zgm-audit` draws the line the same way.
- **Option B was built and measured** (2026-11-11, old and new code alternating, 5 rounds, local database). The number of statements per call grows from 1 to 3, and to 4 with the event written. Throughput relative to the code without a transaction: a flood of refusals 0.52, and 0.35–0.43 with the event written to a table; attempts on many keys 0.58–0.62; sign-in 0.53–0.61. The most expensive operation is the refusal, which is the operation there is most of during an attack.
- **The operations in a transaction are rare or cheap compared with the rest:** `unblock` and a rule change are done by a person, and `reset` comes after a successful sign-in. A single call costs about 0.5–0.7 ms more (`unblock` 0.68 ms without a transaction, 1.40 ms with a transaction and the write).
- **`KeyBlocked` stays after the fact,** because only after the query is it known whether the attempt filled the window; a transaction for it would mean a transaction around every attempt. When this event is lost, the block is still in the table, with its time, and on the list of blocked keys ([ADR-0012](0012-list-of-blocked-keys.md)).
- **Refusals are a signal of abuse.** A refusal deliberately writes nothing to the database, so without an event it would leave no trace at all.
- **The standards in our requirements catalogue (not published) (LOG-01, LOG-02, LOG-07, LOG-12) demand that events are recorded and protected, and that a failure of logging comes to light.** They do not require the write to be in the transaction of the operation; that is our choice.
- **Checked by tests:** the event of a change is in the application's table when the call returns; an audit that returns an error or panics leaves the key blocked and the rule unchanged; a request interrupted while the event is being written leaves neither the change nor the event; 30 attempts at once with a limit of 3 give one `KeyBlocked` and 27 `AttemptRefused`; events contain neither the email nor its hash; for an event after the fact the connection is already back in the pool.

## Consequences

- **The "together or not at all" guarantee holds only when the application writes the event with the connection it was given.** A receiver that sends the event somewhere else does not have this guarantee.
- **An audit failure stops unblocking, `reset` and rule changes.** A successful sign-in by somebody who was blocked then ends with a `reset` error; the application decides what to do about it.
- **`record_in_transaction` is awaited on the path of the request, with a transaction open,** so it should be short: one write.
- **`record_after_the_fact` must return at once.** A receiver that writes to a database hands the event to a queue. A slow one blocks the request's thread: in a measurement, 1.5 seconds in this method stopped another request for that time.
- **The connection goes back to the pool before `record_after_the_fact` through `PoolConnection::return_to_pool` from `sqlx`,** a function that is public but hidden in the documentation: an ordinary drop of the connection hands its return to a background task, which starts only after the report. The `sqlx` version is pinned, and a test will show it if the function stops working after an upgrade.
- **Events after the fact are lost when the application crashes or the request is interrupted between the attempt and the report.**
- **Under a flood of refusals the receiver gets as many events as there are refusals.** It can count them instead of writing each one.
- **A caught panic in `record_after_the_fact` disappears without a trace.**
- **`unblock` of a key that does not exist opens a transaction and rolls it back.**
- **The limit name in an event is a `String`,** because for `unblock` it comes from the database and can be the name of a limit that is no longer in code.
- **The example has `ConsoleAudit`,** which prints events to the console and says which way they came.

## When to revisit

- A requirement from the catalogue or from a customer demands that the blocking of a key (`KeyBlocked`) or a refusal, too, cannot be left without a recorded event.
