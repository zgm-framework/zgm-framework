# ADR-0014: The application builds an `Attempts` object once for a limit and a key, and it has `hit` and `reset`; the attempt is counted before the action

> **Status:** accepted
> **Date:** 2026-11-11
> **Applies to:** `zgm-limits` (`src/attempts.rs`, `Limits::attempts`, `repository::hit_window`, `repository::delete_counter`, `examples/tour/example.rs`)

## Context

- **One sign-in is two calls with the same limit and key:** counting the attempt and forgetting it after a success. Each would repeat the key and compute the hash of the hidden part again.
- **The first version of this decision also had `status()`** and recommended the pattern "ask, check the password, count only failures", so that a successful sign-in would write nothing. A code review showed that this pattern does not limit attempts sent at once: `status()` reserves nothing, so all the requests get past it before the first failure is written. Reproduced with a limit of 3: 200 sign-ins at once gave 200 checked passwords.

## Options considered

**How to give the limit and the key:**

| Option | Pros | Cons |
|---|---|---|
| **A. An object built once, with methods that take no arguments** | the key is given once; the hash is computed once; the enum of limits stays a list of names | the compiler does not check that a per-account limit got an account |
| **B. The key data in the variants of the enum of limits,** e.g. `AppLimit::Login { tenant_id, account_id }` | the limit itself says what key it needs, and the compiler checks it | `strum` will not give the list of limits; a filter by limit needs a second enum; the product writes a second `match` |
| **C. Arguments with every call** | no new type | the key is repeated, the hash is computed with every call |

**The name of the object:** `counter` (the word from the table), `limit` (clashes with the enum, the trait and `Limits`), `guard` (means something else in Rust), `attempts`.

**Whether there is a question without counting (`status`):**

| Option | Pros | Cons |
|---|---|---|
| **A. There is none; there is only `hit`, called before the action** | checking and counting are one statement, so of any number of attempts sent at once only as many pass as there is room for; there is no method that can be misused | a successful sign-in is two writes: inserting the row and deleting it |
| **B. There is `status`, and `hit` is called after a failure** | a successful sign-in writes nothing | attempts sent at once all pass; it looks like the natural "check before you act", so everybody will reach for it |

## Decision

**The object: A, under the name `attempts`, with the tenant given by name. `status`: A, that is, there is none** (decided by the maintainer on 2026-11-10 and 2026-11-11).

```rust
let login_attempts = limits
    .attempts(AppLimit::WrongPassword)
    .tenant_id(tenant_id)
    .subject_id(account_id)
    .call()
    .await?;

if let Answer::Refused { until } = login_attempts.hit().await? {
    return too_many_attempts(until);   // the password is not checked
}
if password_is_right {
    login_attempts.reset().await?;
}
```

- **`limits.attempts(limit)` takes `.tenant_id(...)` (required), `.subject_id(...)`, `.hidden(...)` and `.hidden_exact(...)`.**
- **`.subject_id(...)` with the all-zero UUID is the error `LimitsError::NilSubject`** ([ADR-0005](0005-counters-table-and-key.md)). The counter of the whole tenant is obtained by giving no account.
- **The hash of the hidden part is computed once, when the object is built.** That is why building is `async` and can return an error.
- **`hit()` counts the attempt and is called before the action** that the limit guards. The action is done only after `Answer::Allowed`.
- **`reset()` deletes the row of the counter,** under the hash of every version of the secret key. It returns `true` when it lifted a block that was still on.
- **For `Rule::Bucket`, `hit()` returns an error;** `reset()` works for every way of counting.
- **`Answer` is marked `#[must_use]`.** `attempts.hit().await?;` without using the answer gives a compiler warning, because such code counts the attempt but limits nothing. A code review showed 10 exports that got through a "5 per hour" limit this way.
- **A refusal from `hit()` carries the time from which an attempt may be made again,** so the message "try at 13:00" needs no separate question.

## Why

- **A hard bound matters more than a saved write.** With passwords and codes, what counts is that at most as many attempts get checked as the limit says.
- **The cost at sign-in is negligible:** checking a password is a deliberately slow hash computation, tens of milliseconds, and two small writes are a fraction of a millisecond.
- **A method that does not exist cannot be misused.** Three independent reviewers pointed at `status` before the password check as the biggest risk of the module.
- **`attempts` is in the language of what the module does,** and clashes with no existing name.
- **Checked with a test:** 60 sign-ins at once on one account, with a limit of 3, give exactly 3 checked passwords, in 10 rounds in a row.

## Consequences

- **The pattern for a limit on an account or an e-mail:** `hit`, the password check, `reset` after a right one.
- **An attempt with the right password is counted too,** until `reset` deletes it. When the account is blocked, the right password gets a refusal.
- **On a counter shared by many people, e.g. of an IP address, `reset` after a successful sign-in opens a hole:** somebody with an account of their own resets the counter of the whole address. There, `reset` is not called, and the threshold is set with a margin for successful sign-ins.
- **`reset()` by key and `unblock` from the list of blocked keys do the same thing,** that is, they delete the row. The first is for the application's code, the second for the admin ([ADR-0012](0012-list-of-blocked-keys.md)).
- **The object remembers the hashes from the moment it was built.** It is built for one request and not kept between requests, because after a change of the secret key it would have the old list.
- **The state cannot be asked for without counting,** e.g. to gray out a form for somebody who has not tried yet.

## When to revisit

- An action appears where the state of the limit must be shown before the attempt, and a few attempts over the limit under parallel requests do no harm.
