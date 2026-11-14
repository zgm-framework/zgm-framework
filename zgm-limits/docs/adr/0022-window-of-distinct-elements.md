# ADR-0022: "At most N distinct" is a window filled by distinct elements, with the list of elements in the counter's row

> **Status:** accepted
> **Date:** 2026-11-11
> **Applies to:** `zgm-limits` (`Rule::WindowOfDistinct`, `Attempts::hit_distinct`, `repository::hit_distinct`, `keyed_hash::element_data`)

## Context

- **An ordinary window counts attempts.** It does not see an attack in which one password is tried on many accounts: each account gets one attempt, so the per-account limit never fills.
- **What is needed is counting distinct targets,** e.g. at most 10 distinct accounts from one IP address, or at most so many distinct records opened by one account.

## Options considered

**Where the set of elements lives:**

| Option | Pros | Cons |
|---|---|---|
| **A. In the counter's row, as a list in `state`** | the same table; one query per attempt; the database itself makes sure that N+1 does not get in | the row grows with N; reasonable up to a few hundred elements |
| **B. A separate table, a row per element** | N can be as large as needed | a second table, a second cleanup and a row lock on every attempt |

**How to write it in `Rule`:**

| Option | Pros | Cons |
|---|---|---|
| **A. A separate variant `Rule::WindowOfDistinct { at_most, per }`** | existing rules do not change; the name shows that it is a window; two variants match two queries | two variants with the same fields |
| **B. A third field in `Rule::Window`,** e.g. `counts: Counts::Attempts` | one variant | every ordinary rule writes a field that almost always has the same value |

## Decision

**Set: A. Notation: A** (decided by the maintainer on 2026-11-11: "so add the rule from A").

- **It is still a window.** The first attempt opens it, it lasts `per`, and after its end everything starts over. The only difference is what fills it: every new element, not every attempt.
- **`attempts.hit_distinct(element)`** counts an attempt on an element:
  - the element is already in the window: it passes, nothing is written;
  - the element is not there and there is room: it is added;
  - the element is not there and the window is full: a refusal until the end of the window, nothing is written.
- **The `state` of such a counter is a list of element hashes and the threshold the window opened with,** e.g. `{"elements": ["0100…", "0100…"], "at_most": 10}` ([ADR-0017](0017-tenant-rules-in-the-database.md)).
- **An element reaches the database only as a hash,** computed by `KeyedHash` from the limit name, the tenant and the element, under its own label. The text of the element is passed in unchanged.
- **We look for an element with the hashes of all versions of the secret key,** and we add the hash of the current version. After the key changes, an element from before the change is not counted a second time, as long as the old version can still be read.
- **`hit()` on such a limit and `hit_distinct()` on an ordinary window are the error `LimitsError::WrongWayOfCounting`,** with the name of the method that has to be used.
- **A tenant overrides the numbers, but not the kind.** A rule of a kind other than the default is the error `LimitsError::RuleOfAnotherKind`: whether a limit counts attempts or elements is decided by the code that calls `hit` or `hit_distinct`.
- **The rest works unchanged:** `reset()` deletes the whole window, a full window has `blocked_at` and is on the list of blocked keys, and the `KeyBlocked` and `AttemptRefused` events are the same.

## Why

- **Limits of this kind have a small N,** and a list in the row keeps what the module is built on: one query, and a refusal without a write and without a lock ([ADR-0006](0006-counting-in-one-sql-statement.md)).
- **The table did not need a change,** because from the start `state` has been the place for whatever a way of counting remembers ([ADR-0005](0005-counters-table-and-key.md)).
- **The condition "there is room and the element is not there yet" also stands at the write itself.** Checked by tests: 40 distinct elements at once with a limit of 3 leave exactly 3 in the window, and the same element sent 40 times at once takes one place.
- **The tests detect both bugs that were introduced on purpose to check them:** removing the condition at the write, and looking for an element only with the hash of the current key version.

## Consequences

- **"Blocked" here means: new elements will not pass.** Elements that are already in the window keep passing. The list of blocked keys does not show this difference.
- **The row grows by a dozen or so bytes per element** and is rewritten on every new one.
- **A window of distinct elements holds at most 1000** (`MAX_DISTINCT_ELEMENTS`, decided by the maintainer on 2026-11-11). A limit whose rule in code or whose upper bound for a tenant exceeds 1000 stops `build()` with a panic. This is a logical bound of the mechanism, not a range from a standard. A window of attempts has no such bound.
- **Measured at 1000 elements** (2026-11-11, local database): a new element into an almost full window 2.2–2.5 ms, against 1.5 ms into an empty one; a known element and a refusal on a full window 1.1 ms; `state` takes 17.5 kB; 32 concurrent callers with 3000 distinct elements let through exactly 1000, with no `NoAnswer` at all, the slowest call 0.2 s. A larger N will need a separate table and a separate decision.
- **Every `hit_distinct` computes the hash of the element,** which means it calls `KeyedHash` once per attempt. The hash of the key is still computed once, when `Attempts` is built.
- **When the old version of the secret key disappears in the middle of a window,** an element from before the change is counted a second time. The limit is then sometimes stricter, never looser.
- **The example has the `AccountsPerAddress` limit:** three distinct accounts from one address per ten minutes.
- **This is the first second way of counting in the module.** The choice of query by the kind of rule is in `Attempts`, and the `tenant_rules` table stores two kinds.
- **A counter written by another way of counting gives no answer** (`NoAnswer`) until the end of its window. The module does not repair or recognize this (decided by the maintainer on 2026-11-11): changing the way of counting under the same name is a change in the product's code, which is avoided by giving the limit a new name, and handling such a counter would be extra code in the query of every attempt. The README covers this in "What to watch out for".
