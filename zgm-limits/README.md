# zgm-limits

Attempt limits for Rust applications on PostgreSQL: how many times something may be done in a given time. Three wrong passwords per 15 minutes, three sign-in codes per e-mail address, five exports per hour for a whole tenant, ten different accounts tried from one IP address.

The counters live in the application's own database, so a limit holds across every copy of the application and needs no Redis and no separate service.

Part of [zgm-framework](../README.md), a collection of modules used by [leads.run](https://leads.run).

> **Status:** version 0.1, before the first release. The API may still change. Every design choice is written down in [docs/adr](docs/adr/README.md).

- **One SQL statement per attempt.** Checking the limit and counting the attempt are the same statement, so of any number of attempts sent at once only the allowed number pass.
- **A refusal writes nothing.** A flood of attempts on a blocked key costs the database one read each.
- **Keys by account, by a hidden value, or for a whole tenant.** A hidden value (an e-mail, an IP address) reaches the database only as a keyed hash.
- **Distinct elements.** "At most N different accounts from one address" next to "at most N attempts".
- **Rules in code, overridable per tenant** within bounds the product sets.
- **A list of blocked keys** per tenant, and unblocking.
- **Audit events** instead of logs; the event of a change is written in the transaction of that change.
- **It cleans up after itself and creates its own tables.**

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Guide](#guide)
  - [Limits and rules](#limits-and-rules)
  - [Counting an attempt](#counting-an-attempt)
  - [Whose attempt it is: the key](#whose-attempt-it-is-the-key)
  - [Distinct elements](#distinct-elements)
  - [Blocked keys](#blocked-keys)
  - [Rules of a tenant](#rules-of-a-tenant)
  - [Events](#events)
  - [Errors](#errors)
- [Connection pool](#connection-pool)
- [What to watch out for](#what-to-watch-out-for)
- [How it works](#how-it-works)
- [How it was tested](#how-it-was-tested)
- [Running the example and the tests](#running-the-example-and-the-tests)
- [Not there yet](#not-there-yet)

## Install

Requirements: PostgreSQL 18, `tokio`, `sqlx` 0.9, Rust 1.94.

```toml
[dependencies]
zgm-limits = { git = "https://github.com/zgm-framework/zgm-framework", tag = "zgm-limits-v0.1.0" }

# The crate's traits and derives use these:
async-trait = "0.1"
strum = { version = "0.28", features = ["derive"] }
```

## Quick start

Everything below is taken from the example in [`examples/tour`](examples/tour), which runs every feature of the crate from start to end.

**1. List the limits in an enum and give each a rule.**

```rust
use zgm_limits::{Count, Limit, Period, Rule};

#[derive(Debug, Clone, Copy, strum::IntoStaticStr, strum::VariantArray, strum::EnumProperty)]
#[strum(serialize_all = "snake_case")]
pub enum AppLimit {
    WrongPassword,
    Export,
}

impl Limit for AppLimit {
    fn rule(self) -> Rule {
        match self {
            Self::WrongPassword => Rule::Window { at_most: Count::new(3), per: Period::minutes(15) },
            Self::Export => Rule::Window { at_most: Count::new(5), per: Period::hours(1) },
        }
    }
}
```

**2. Build the module once, when the application starts.**

```rust
use strum::VariantArray;
use zgm_limits::{JobError, Limits};

zgm_limits::migrate(&pool).await?;       // creates and updates the module's tables

let limits = Limits::builder()
    .pool(pool)
    .keyed_hash(hmac)                    // your HMAC, see "Whose attempt it is"
    .audit(audit)                        // your audit, see "Events"
    .limits(AppLimit::VARIANTS)
    .run_jobs_in_background(|error: JobError| eprintln!("{}: {}", error.job, error.error))
    .build();
```

**3. Count the attempt before the action the limit guards.**

```rust
let attempts = limits
    .attempts(AppLimit::WrongPassword)
    .tenant_id(tenant_id)
    .subject_id(account_id)
    .call()
    .await?;

if let Answer::Refused { until } = attempts.hit().await? {
    return Ok(SignIn::TooManyAttempts { until });   // the password is not even checked
}
if password_is_right {
    attempts.reset().await?;                        // forget the attempts
}
```

That is a complete sign-in limit. `limits` is cheap to clone; give a clone to every handler.

## Guide

### Limits and rules

A limit is a variant of your enum. Its name in the database is the variant's name in snake case (`wrong_password`), and its rule is what `Limit::rule` returns. Nothing is registered anywhere else: a new limit is a new variant and a new arm of the `match`.

| Rule | What it counts |
|---|---|
| `Rule::Window { at_most, per }` | attempts; the first attempt opens a window that lasts `per`, and `at_most` attempts fit in it |
| `Rule::WindowOfDistinct { at_most, per }` | different elements in the same kind of window; see [Distinct elements](#distinct-elements) |
| `Rule::Bucket { capacity, refill_every }` | a bucket with tokens; **not counted yet**, `hit()` returns an error for it |

- **`Count` and `Period` cannot hold zero.** `Count::new(0)` and `Period::seconds(0)` panic, and in a `const` they do not compile. A rule that would let nothing through cannot be written.
- **Rules are read when the module is built.** `build()` reads the rule of every limit on the list, so a mistake in a rule stops the application at startup, not at the first attempt in that limit ([ADR-0016](docs/adr/0016-rules-checked-at-build.md)).
- **The window is fixed, from the first attempt.** A key that used its three attempts at 12:00 is refused until 12:15, whatever it does in between ([ADR-0002](docs/adr/0002-window-from-the-first-attempt-and-rule-as-an-enum.md)).

All four steps of the builder are required: `keyed_hash`, `audit`, `limits`, `run_jobs_in_background`. Leaving one out does not compile. An optional `.clock(...)` replaces the system clock in tests.

### Counting an attempt

`limits.attempts(limit)` describes whose attempts in which limit; it is built once and used for every call about that key.

| Method | What it does |
|---|---|
| `hit()` | counts one attempt and returns `Answer::Allowed` or `Answer::Refused { until }` |
| `hit_distinct(element)` | the same for a limit that counts distinct elements |
| `reset()` | forgets the key, e.g. after a sign-in that succeeded; returns `true` when it lifted a block that was still on |

- **Call `hit()` before the action, and act only on `Allowed`.** The check and the count are one statement. "Ask first, count only the failures" would let through every attempt sent at the same moment, so there is deliberately no method that asks without counting ([ADR-0014](docs/adr/0014-attempts-object-hit-reset.md)).
- **The answer must be looked at.** `attempts.hit().await?;` alone counts the attempt and limits nothing; `Answer` is `#[must_use]`, so the compiler warns.
- **An error is a refusal, not a permission.** When `hit()` returns an error, do not let the action through.
- **Call `reset()` only on a key with one owner.** On a counter shared by many people, such as an IP address, never.

```rust
pub async fn sign_in(limits: &Limits, tenant_id: Uuid, account_id: Uuid, password: &str)
    -> Result<SignIn, LimitsError>
{
    let attempts = limits
        .attempts(AppLimit::WrongPassword)
        .tenant_id(tenant_id)
        .subject_id(account_id)
        .call()
        .await?;

    if let Answer::Refused { until } = attempts.hit().await? {
        return Ok(SignIn::TooManyAttempts { until });
    }
    if password == RIGHT_PASSWORD {
        attempts.reset().await?;
        Ok(SignIn::SignedIn)
    } else {
        Ok(SignIn::WrongPassword)   // already counted; the third one blocks the account
    }
}
```

### Whose attempt it is: the key

`tenant_id` is required. The rest decides what the counter is about.

```rust
// An account (or anything else with a UUID). Stored as it is.
limits.attempts(AppLimit::WrongPassword).tenant_id(tenant_id).subject_id(account_id)

// A hidden value: an e-mail, an IP address. Stored only as a keyed hash.
limits.attempts(AppLimit::SignInCode).tenant_id(tenant_id).hidden(&email)

// A hidden value taken exactly as written, e.g. the identifier of an API key.
limits.attempts(AppLimit::ApiCalls).tenant_id(tenant_id).hidden_exact(&api_key_id)

// Neither: one counter for the whole tenant.
limits.attempts(AppLimit::Export).tenant_id(tenant_id)
```

- **`hidden` makes the text uniform first:** lower case, no whitespace, no invisible characters such as a zero-width space. `Anna@Example.com ` and `anna@example.com` are one key, so changing the case does not get around the limit. `hidden_exact` changes nothing.
- **`hidden` does not understand e-mail addresses.** `anna+1@example.com` and `anna+2@example.com` are two keys. Canonicalize such addresses yourself, or add a limit by IP address.
- **`subject_id` and a hidden value can be combined** into one key.
- **The nil UUID is not an account.** It is how the table marks "no account", so `subject_id(Uuid::nil())` is an error; for the whole tenant give no `subject_id`.

The hash comes from your implementation of `KeyedHash` ([ADR-0013](docs/adr/0013-keyed-hash-from-the-application.md)):

```rust
#[async_trait]
impl KeyedHash for AppHmac {
    async fn hashes(&self, data: &[u8]) -> Result<Vec<Vec<u8>>, KeyedHashError> {
        // One HMAC of `data` for every version of the secret key that can still be read,
        // the current version first.
        Ok(self.keys.iter().map(|key| hmac_sha256(key, data)).collect())
    }
}
```

- **It must be a hash with a secret key,** such as HMAC-SHA-256. A plain SHA-256 compiles too, but then anybody with a copy of the database finds the IP addresses in minutes by trying them all.
- **The module holds no key.** The method is `async` and may fail, so an implementation may fetch its key; its error is the error of the attempt (`LimitsError::KeyedHash`).
- **Changing the secret key does not reset counters.** Return a hash for every version that can still be read: the module looks a counter up by all of them and writes new counters with the first. Remove an old version only after the longest window has passed ([ADR-0008](docs/adr/0008-changing-the-secret-key.md)).
- **What goes into the hash** is the limit name, the tenant, the account and the hidden value together, so the same e-mail gives another hash in every limit and every tenant. A hash may be at most 128 bytes long.

### Distinct elements

Some abuse is invisible to a limit per account: one password tried on a thousand accounts gives each account a single attempt. A limit that counts *different elements* catches it.

```rust
// the rule: at most ten different accounts tried from one address in an hour
Self::AccountsPerAddress => Rule::WindowOfDistinct { at_most: Count::new(10), per: Period::hours(1) },

// around the action: the address is the key, the account is the element
let from_address = limits
    .attempts(AppLimit::AccountsPerAddress)
    .tenant_id(tenant_id)
    .hidden(&ip_address)
    .call()
    .await?;
if let Answer::Refused { until } = from_address.hit_distinct(&login).await? {
    return Ok(SignIn::TooManyAttempts { until });
}
```

- **An element that is already in the window always passes** and changes nothing. Only a new element needs room.
- **"Blocked" here means: no new elements.** The ones already in the window keep passing.
- **The element reaches the database only as a hash,** like a hidden value.
- **At most 1000 elements per window** (`MAX_DISTINCT_ELEMENTS`). The elements lie in the counter's row, and with thousands of them an attempt gets slow, so a larger rule stops `build()` ([ADR-0022](docs/adr/0022-window-of-distinct-elements.md)).

### Blocked keys

A key is blocked when its window is full. An administrator can list the blocked keys of a tenant and unblock one ([ADR-0012](docs/adr/0012-list-of-blocked-keys.md)).

```rust
let page = limits.get_blocked().tenant_id(tenant_id).call().await?;
for blocked in &page.items {
    println!("{} of {} until {}", blocked.limit_name, blocked.subject_id, blocked.blocked_until);
}

// Narrowed, and the next page:
let export: &'static str = AppLimit::Export.into();
let page = limits
    .get_blocked()
    .tenant_id(tenant_id)
    .limit_name(export)             // one limit only
    .whole_tenant(true)             // only counters of the whole tenant
    .include_expired(true)          // also blocks that are over but not cleaned up yet
    .maybe_after(page.next_cursor)  // the page after this one
    .call()
    .await?;

limits.unblock(tenant_id, blocked.id).await?;   // the key starts from nothing
```

- **Pages of 100, the newest block first.** `next_cursor` is `Some` when there is more.
- **A hidden value is never shown.** The list has only its hash (`key_hash`), and `whole_tenant` says when the counter is of the whole tenant.
- **An `id` of another tenant unblocks nothing.** Every query of the module is scoped to a tenant.

### Rules of a tenant

The rules in the code are the defaults. A tenant may replace the numbers of a rule, within bounds the product writes over the limit as four `strum` properties: all four or none ([ADR-0017](docs/adr/0017-tenant-rules-in-the-database.md), [ADR-0018](docs/adr/0018-override-bounds-in-strum-attributes.md)).

```rust
pub enum AppLimit {
    // A tenant may set 1 to 20 attempts per one minute to one day.
    #[strum(props(min_at_most = 1, max_at_most = 20, min_per_seconds = 60, max_per_seconds = 86400))]
    WrongPassword,
    // No bounds: a tenant cannot change it.
    Export,
}
```

```rust
limits.get_defaults();                                    // the rules from the code, with bounds
limits.get_rules().tenant_id(tenant_id).call().await?;    // the rules in force in a tenant

limits
    .override_rule(AppLimit::WrongPassword)
    .tenant_id(tenant_id)
    .rule(Rule::Window { at_most: Count::new(10), per: Period::hours(1) })
    .call()
    .await?;

limits.restore_default(AppLimit::WrongPassword).tenant_id(tenant_id).call().await?;
```

- **A rule outside the bounds is an error** (`RuleOutOfBounds`), and so is a rule for a limit with no bounds (`RuleCannotBeChanged`). Giving only some of the four properties stops `build()`.
- **A tenant changes numbers, not the way of counting.** Whether a limit counts attempts or distinct elements is in the code.
- **A new rule is for windows opened from then on.** A window that is already open counts to its end by the rule it was opened with; its `at_most` lies in the counter next to the count. A change deletes nothing and lifts no block; an administrator unblocks from the list.
- **The module knows no standard or regulation.** It enforces only what makes a rule work at all. The values a standard asks for are set by whoever configures the system.

### Events

The module has no logging of its own. Wherever a line of a log would be, it reports an event to the `Audit` you give the builder ([ADR-0021](docs/adr/0021-events-instead-of-logs.md)).

```rust
#[async_trait]
impl Audit for AppAudit {
    // A change somebody made: written in the transaction of that change.
    async fn record_in_transaction(
        &self,
        transaction: &mut PgConnection,
        event: LimitsEvent,
    ) -> Result<(), AuditError> {
        sqlx::query("INSERT INTO app_audit (event) VALUES ($1)")
            .bind(format!("{event:?}"))
            .execute(transaction)
            .await
            .map_err(AuditError::new)?;
        Ok(())
    }

    // A refusal, a key that got blocked, a pass of the cleanup: like a line of a log.
    fn record_after_the_fact(&self, event: LimitsEvent) {
        self.queue.send(event).ok();
    }
}
```

| Event | When | How it arrives |
|---|---|---|
| `BlockLifted { key, by }` | a block that was still on was lifted, by `reset()` or `unblock()` | in the transaction |
| `RuleChanged { limit_name, tenant_id, rule }` | a tenant changed a rule or went back to the default | in the transaction |
| `AttemptRefused { key, until }` | an attempt was refused | after the fact |
| `KeyBlocked { key, until }` | an attempt passed and filled its window; from now on the key is blocked | after the fact |
| `ExpiredDeleted { counters }` | a pass of the cleanup deleted rows | after the fact |

- **The event of a change is written together with the change, or not at all.** `reset()`, `unblock()`, `override_rule` and `restore_default` open a transaction, make the change, and call `record_in_transaction` with that transaction before the commit. What you write with the connection you are given is part of it.
- **An audit that fails takes the change back.** An error or a panic in `record_in_transaction` comes back as `LimitsError::Audit`, and the block or the rule stays as it was. An account cannot be unblocked and a rule cannot be changed without its event, also when the request is dropped halfway.
- **Attempts report after the fact.** `hit()` stays one statement with no transaction around it. `record_after_the_fact` is called after it, when the connection is back in the pool; it cannot fail, and a panic in it is caught. It must return at once: hand the event to a queue.
- **An event after the fact can be lost** when the application stops between the attempt and the report. The block itself is still in the table and on the list of blocked keys.
- **Why not everything in a transaction?** We built it and measured it: a transaction around every attempt took the throughput down to 35–60%, and the most expensive call became the refusal, which is the most frequent one during an attack.
- **Ordinary work reports nothing:** an attempt that passed without blocking, a `reset()` with no block to lift, a pass of the cleanup that found nothing.
- **No event holds a hidden value or its hash,** only `has_hidden_part`.
- **During a flood there are as many events as refusals.** Count them instead of storing each.
- **`KeyBlocked` followed at once by `BlockLifted { by: Reset }` on the same key is usually a successful sign-in, not an incident.** The attempt is counted before the password is checked, so the right password on the last allowed attempt first fills the window and the `reset()` after it lifts the block.

### Errors

A refused attempt is not an error; it is `Answer::Refused`. `LimitsError` is for what went wrong.

| Error | Meaning |
|---|---|
| `Database` | the database failed, or the pool gave no connection in time |
| `Migration` | a migration of the module's schema failed |
| `NoAnswer { tries }` | the attempt got no answer in ten tries because other attempts kept changing its counter; treat it as refused |
| `KeyedHash`, `KeyedHashGaveNoHash`, `KeyedHashGaveLongHash` | your `KeyedHash` failed, returned nothing, or returned more than 128 bytes |
| `Audit` | your `Audit` failed or panicked in `record_in_transaction`; the change was rolled back |
| `NilSubject`, `HiddenAndHiddenExact` | a key that cannot be: the nil UUID as an account, or both kinds of hidden value |
| `UnknownLimit` | the limit was not on the list given to the builder |
| `WrongWayOfCounting` | `hit` on a limit of distinct elements, or `hit_distinct` on a limit of attempts |
| `RuleCannotBeChanged`, `RuleOutOfBounds`, `RuleOfAnotherKind` | a tenant's rule the limit does not allow |
| `BucketNotImplemented` | the limit has a bucket rule, which is not counted yet |

## Connection pool

The module borrows connections from the application's pool. We recommend building it like this ([ADR-0020](docs/adr/0020-recommended-pool-settings.md)):

```rust
let pool = PgPoolOptions::new()
    .test_before_acquire(false)
    .idle_timeout(Duration::from_secs(120))
    .connect(&database_url)
    .await?;
```

- **`test_before_acquire(false)` saves one round trip on every call.** By default `sqlx` checks a connection before handing it out, so every `hit()` is two round trips instead of one. During a flood of refusals, where the query itself is very cheap, turning the check off gave 10–65% more refusals per second in our measurement. On an ordinary sign-in the difference does not show.
- **`idle_timeout(120 s)` closes connections before they die.** A firewall or a load balancer may drop a connection that lies unused. Set this shorter than your network drops idle connections.
- **What you give up:** when a connection dies anyway, e.g. at a database restart, one request per dead connection gets `LimitsError::Database`. Refuse the action then.
- **These are settings of the whole pool,** so they apply to the application's own queries too.

### Two database settings

An attempt that passes writes the counter's row. When somebody else holds that row, the attempt waits for it with no limit, holding a pool connection. Refusals do not wait: they write nothing. We recommend ([ADR-0023](docs/adr/0023-the-database-bounds-waiting-for-a-row.md)):

```sql
ALTER DATABASE app SET lock_timeout = '2s';
ALTER DATABASE app SET idle_in_transaction_session_timeout = '60s';
```

- **`lock_timeout` bounds the one who waits.** The database cancels a statement that waits for a row longer than this; the attempt ends with `LimitsError::Database` and its connection goes back to the pool.
- **`idle_in_transaction_session_timeout` bounds the one who holds.** The database disconnects a session that opened a transaction and does nothing in it, e.g. a forgotten `psql` window.
- **The module sets neither** and works without them. Without them, one transaction left open on a counter's row can take every connection of the pool with attempts on that one key.
- **2 and 60 seconds are starting values.** The settings apply to every session of the database.

## What to watch out for

- **An error of the module is a refusal, not a permission.** When `hit()` returns an error, do not let the guarded action through.
- **A request cancelled mid-call may still use up an attempt.** Once the statement is sent, the database finishes it; see [Measurements](#measurements).
- **A full pool means waiting.** A request waits for a connection for up to 30 seconds by default before it gets an error; the pool's `acquire_timeout` sets that.
- **A block works against the victim too.** Whoever knows a login can keep that account blocked for ever with three wrong passwords every 15 minutes. Every lock-out by account works like this; an administrator sees it on the list of blocked keys and can lift it.
- **`reset()` after a success suits a key with one owner.** On an account its owner signs in to often, an attacker gets fresh attempts after each of those sign-ins.
- **`Refused { until }` tells when the window began.** The application decides whether to show that time to the user.
- **A long-open transaction in the same database slows counting.** While it lasts, PostgreSQL cannot clean up old versions of counters that change often; in our measurement attempts on hot keys slowed down by about half. The cleanup slows down too, from about 300 to 80–170 rows per second. This is true of any transaction in the database, not only the module's.
- **A change of a rule does not act at once on keys with an open window.** Tightening a limit does not stop somebody in the middle of a window, and loosening it does not unblock anybody. This lasts at most as long as the window.
- **When you change the way a limit counts, give it a new name.** A limit that counted attempts and now counts distinct elements (or the other way round) finds, under the old name, counters it cannot read: keys with an open window get `NoAnswer` until that window ends.

## How it works

**The table.** One migration creates the schema `zgm_limits` with two tables ([ADR-0005](docs/adr/0005-counters-table-and-key.md)).

- **`counters`: one row per key in one limit.** The key is `limit_name`, `tenant_id`, `subject_id` and `key_hash`; no part is NULL (no account is the nil UUID, no hidden value is empty bytes), so it is a plain primary key.
- **`state` (`jsonb`) holds what the way of counting remembers.** For a window: the count and the `at_most` the window was opened with. For a window of distinct elements: the hashes of the elements and that `at_most`.
- **`expires_at` is the end of the window,** after which the row means nothing. **`blocked_at`** is set by the attempt that fills the window.
- **`tenant_rules`: one row per tenant and limit,** only for rules a tenant changed. The defaults are not in the database.

**Counting.** One statement reads the row, writes only when there is room, and returns "allowed" or "refused until the end of the window". A refusal writes nothing and locks nothing ([ADR-0006](docs/adr/0006-counting-in-one-sql-statement.md)). When other attempts change the counter between the read and the write, the statement gives no answer and is asked again, up to ten times.

**Cleanup.** `build()` starts a loop in the background ([ADR-0015](docs/adr/0015-cleanup-in-a-job-loop.md)). Every 2 minutes it deletes, in batches of 100 with a pause of 300 ms between them, the counters whose `expires_at` passed at least an hour ago; the hour is room for clock differences between servers. The application calls nothing. An error of a pass goes to the function given to `run_jobs_in_background` and does not stop the loop. The loop ends with the last clone of `Limits`, and it is safe with several copies of the application running at once.

**Migrations.** `zgm_limits::migrate(&pool)` creates and updates the module's tables and keeps its own migration history in its schema, so it does not mix with the application's migrations ([ADR-0019](docs/adr/0019-the-module-runs-its-migrations.md)).

**The code.** `src/limits.rs` is the entry point, `src/attempts.rs` the attempts of one key, `src/rule.rs` the rules, `src/audit.rs` the events, `src/jobs.rs` the cleanup loop, and `src/repository.rs` holds every SQL statement of the crate.

## How it was tested

- **94 tests on a real PostgreSQL,** each on its own empty database ([ADR-0010](docs/adr/0010-tests-in-the-tests-folder-on-a-database.md)).
- **Under a flood, twice,** on a table of about 2 million counters: about 2 million attempts sent straight as SQL and about 475,000 calls through the Rust API, with rule changes, resets, unblocks and the cleanup running at the same time. No key was ever allowed more than its limit, refusals wrote nothing, there were no deadlocks, and no attempt needed more than 7 of its 10 tries.

### Measurements

**Read these as orders of magnitude, not as a benchmark.** They come from one laptop (Apple M4, 10 cores, 16 GB) with PostgreSQL 18 in Docker on the same machine, which was busy with other work during the runs (load average 10–18). One round trip to the database (`SELECT 1` through the pool) took 0.38 ms there; between two real machines it will take longer.

One call at a time, no concurrency (median of 2000 calls):

| Call | p50 |
|---|---|
| an attempt that passes | 0.82 ms |
| an attempt that is refused | 0.63 ms |

Under concurrency, through the Rust API, on a table of about 1 million counters (median of five 10–15 s rounds):

| Load | Pool / tasks | Calls per second | p50 | p99 |
|---|---|---|---|---|
| refusals on blocked keys | 10 / 64 | 4,776 | 11 ms | 44 ms |
| refusals on blocked keys | 32 / 256 | 6,471 | 33 ms | 130 ms |
| passing attempts, 200,000 keys | 10 / 64 | 3,364 | 16 ms | 64 ms |
| passing attempts, 200,000 keys | 32 / 256 | 5,093 | 46 ms | 182 ms |
| passing attempts, 20 hot keys | 10 / 64 | 3,642 | 16 ms | 58 ms |
| passing attempts, 20 hot keys | 32 / 256 | 4,343 | 55 ms | 142 ms |
| passing attempts, 2 hot keys | 32 / 256 | about 1,900 | 123 ms | 280–320 ms |

With several times more tasks than connections, most of that latency is a task waiting for a connection from the pool, not the query. Twenty hot keys cost about as much as 200,000 independent ones; the price of contention shows only when nearly every attempt is on the same two rows.

Waiting for a row lock, measured at the SQL level with 32 sessions on a table of 2 million counters, by sampling `pg_locks` and logging every wait above 100 ms:

| Situation | Sessions waiting at once | Longest wait |
|---|---|---|
| everything at once for 3 × 60 s: attempts of both kinds, resets, unblocks, a rule change every 50 ms, two cleanup loops | at most 3 of 32 | 138 ms |
| a tenant under a flood while its rule changes 900–2,200 times in 15 s | none | none |
| a key that comes back while the cleanup deletes its old row | at most 2 of 32 | 352 ms |
| an outside transaction holds a counter's row for 3 s | the attempts on that key | 1.8 s for an attempt that passes; a refusal on the same row answered in 17 ms |

- **In the first row, what waited was two unblocks of the same key, or two attempts on the same key.** Nothing was seen waiting behind the cleanup or behind a rule change. There were 0 deadlocks and 0 failed statements, and 83 of 425,767 attempts (0.02%) had to be asked again.
- **The last row is why we recommend `lock_timeout`** ([Connection pool](#connection-pool)): the module does not bound that wait itself.

**Cancellation.** A request that is cancelled or times out mid-call may still use up an attempt. An attempt is one statement that the database commits on its own. Cancelled while waiting for a pool connection, nothing was sent and nothing is counted. Cancelled after the statement was sent, the database still finishes it, so the attempt counts although the caller never saw the answer. The module can count an attempt nobody used; it never lets more than the limit through. In a run of 12,000 calls under timeouts of 0.2–6 ms, 8,427 were dropped mid-call: the counters were exact and no key went over its limit.

**Not measured:** lock waits sampled during the hot-key runs through the Rust API (only their latency), how long the row lock of one attempt is held, cancelling a request exactly while its statement waits for a row lock, a real network, and a quiet machine.

## Running the example and the tests

The example and the tests need an empty PostgreSQL 18 database in `DATABASE_URL`. Commands are in the [`justfile`](justfile).

```bash
docker run -d --name limits-postgres -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:18
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres

just          # runs the tour: cargo run --example tour
just test     # the tests, each on a new, empty database
just build    # only compiles
```

The tour prints what it does, step by step: wrong passwords that block an account, the list of blocked keys and an unblock, a key by e-mail address, a limit for a whole tenant, distinct accounts from one address, a tenant changing a rule, the errors, and what ended up in the application's audit table.

## Not there yet

- **Counting a bucket with tokens** (`Rule::Bucket`). The variant is in the code, but `hit()` returns an error for it and a tenant cannot choose it ([ADR-0004](docs/adr/0004-bucket-and-escalating-block-postponed.md)).
- **A rule with an escalating block,** as when unlocking a phone.
- **Several rules in one limit,** such as "1 per minute and 5 per hour". Today that is two limits and two calls of `hit()`.
- **More than 1000 distinct elements in a window.**
- **A separate database account for migrations.** Today one account creates the tables and uses them.
- **A check at startup that the migrations are applied.** Today a missing table shows as a database error at the first query.
- **Events of attempts written together with the attempt.** Today only the events of changes are written in a transaction.

## Decisions

Every choice in this crate has a record in [docs/adr](docs/adr/README.md): the context, the options considered with their costs, the decision and why. The records describe the decisions in force today.

## Security

Please report vulnerabilities privately; see [SECURITY.md](../SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
