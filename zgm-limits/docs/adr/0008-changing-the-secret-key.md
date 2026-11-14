# ADR-0008: After the secret key changes, we look for the counter with the hashes of all versions and rewrite nothing

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`repository::hit_window`)

## Context

- **The hash of a key with a hidden part depends on the version of the secret key** ([ADR-0005](0005-counters-table-and-key.md)). After the key changes, the same email gives a different hash, so a search with one hash will not find the counter from before the change.
- **Keys are changed most often when something is wrong,** e.g. during an attack. At that moment the change must not hand the attacker fresh limits (the maintainer, 2026-11-10).
- **A counter found by an old version could be rewritten to the current one,** and rows merged when a key has two. That is a lot of code and extra statements on the path of an attempt.
- **Keys are managed by another module,** which knows nothing about limits or the lengths of windows.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. Search by all versions, rewrite nothing** | counters survive a key change; old rows disappear by themselves after `expires_at` | the query chooses which row to count in |
| **B. Search by all versions and rewrite** | old hashes disappear sooner | rewriting and merging rows |
| **C. Search with the current version only** | one hash, the simplest query | every key change resets the counters with a hidden part |

## Decision

**A** (decided by the maintainer on 2026-11-10).

- **The application gives one function `hashes(data)`** (the `KeyedHash` trait, [ADR-0013](0013-keyed-hash-from-the-application.md)), which returns a hash for every version of the key that can still be read, the current one first. The limits know nothing about keys or versions.
- **The query looks for an open window through `key_hash = ANY(list)`.**
- **An attempt is counted in the open window, whichever hash it lies under.** When there is no window, the new row gets the first hash on the list, that is, the current version.
- **A key without a hidden part has a list with one empty hash** and does not notice a key change.
- **The keys module knows nothing about limits.** The limits take whatever list of versions they are given at the moment.

## Why

- **Rows expire by themselves,** so old hashes disappear without rewriting, after the longest window at the latest.
- **A missing old version breaks nothing.** The counter under its hash is not found and the window starts again. Unlike with encryption, nothing is lost, so the limits have nothing to require from the keys module.
- **Checked on a database:** 3 attempts with the old version and 4 with both versions gave 5 passed and 2 refusals in one row with the old hash; after the end of the window the new row got the current hash; 40 attempts at once with two versions gave exactly 5 passed.

## Consequences

- **An old version of the key is best removed only after the time of the longest window.** Removed earlier, it resets the counters with a hidden part, as option C does. This is advice for whoever changes the keys, not a condition in the code.
- **Two servers with different current versions may create two rows for one key at the same moment.** That person then has up to 2N attempts in one window. This happens only during a key change and only for a key with no open window.
- **When there are two open windows, the attempt is counted in the one with the newer version.**

## When to revisit

- Blocks arrive that last longer than the time for which old versions of the key stay readable.
