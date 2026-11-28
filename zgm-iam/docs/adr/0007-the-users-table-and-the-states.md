# ADR-0007: A user is a UUID v7 under a tenant, with a state and no name; a tenant and a user are `active`, `suspended` or `pending_deletion`, kept as text

> **Status:** accepted
> **Date:** 2026-11-21
> **Applies to:** `zgm-iam` (`migrations/`, the `zgm_iam.users` table, the `state` and `deletes_at` columns of `zgm_iam.tenants`, `src/state.rs`, `Tenant`, `Cargo.toml`)

## Context

- **Everything lives under a tenant** ([ADR-0002](0002-the-tenants-table.md)), users too.
- **In AWS IAM a user has a name because the user signs in with it:** "IAM users sign-in using their account ID or alias, their user name, and a password." The name is unique in the account and case does not tell names apart: "you cannot create resources named both "MyResource" and "myresource"." It may be an e-mail: "IAM user names can be either friendly names, such as Zhang, or email addresses such as zhang@example.com."
- **Here a separate module signs users in** and keeps what a user signs in with, such as an e-mail. The application keeps the name and the e-mail for contact. This module gets a user's UUID.
- **A tenant and a user can be blocked, and are deleted only after a window.** The maintainer: "not immediately, it just gets blocked as inactive, and it can be activated back", and "so that a tenant or a user can be deactivated, or blocked".
- **An AWS account has five states:** `PENDING_ACTIVATION`, `ACTIVE`, `SUSPENDED`, `PENDING_CLOSURE`, `CLOSED`. `PENDING_ACTIVATION`: "the account sign-up process was initiated but never completed". `SUSPENDED`: "the account is unusable because AWS has restricted access". `PENDING_CLOSURE`: the closing has started and the account still works. `CLOSED`: for 90 days "you cannot access AWS services, but you can contact Support to reinstate the account", then "the account is permanently closed".

## Options considered

**What a user has besides its id and its tenant:**

| Option | Pros | Cons |
|---|---|---|
| **A. Nothing** | a copy of the database names nobody; no call of the cipher | the application keeps what it calls a user |
| **B. A name encrypted by the cipher, not unique** | the module can show a name | the module uses it for nothing; the application has the name anyway |
| **C. A name unique in the tenant, through a keyed hash next to the ciphertext** | the database refuses a second "anna@kowalski.pl" in one tenant; the module can find a user by name | a second trait from the application; a rule for upper and lower case; every version of the hash key to look up; the hash must take in the tenant, or the same e-mail in two tenants gives the same hash and a copy of the database shows one person in both |

**What happens to the users of a deleted tenant:**

| Option | Pros | Cons |
|---|---|---|
| **A. `ON DELETE RESTRICT`** | a mistaken `DELETE` does not remove a company with all its users | deleting a tenant first deletes everything under it, in the right order |
| **B. `ON DELETE CASCADE`** | nothing of a deleted tenant stays behind | a `DELETE` removes a company with all its users at once |

**Who keeps every link between rows inside one tenant** (for the tables to come, such as the members of a group):

| Option | Pros | Cons |
|---|---|---|
| **A. The database:** `UNIQUE (tenant_id, id)` on users and foreign keys of pairs, `(tenant_id, user_id)` | a mistake in the code cannot link two tenants | one more index, and foreign keys of pairs in every table that links rows |
| **B. The code:** a query that links two rows looks for both in the tenant | nothing more in the schema | a plain `INSERT ... VALUES` links anything it gets |

**The states:**

| Option | Pros | Cons |
|---|---|---|
| **A. The five states of an AWS account** | the same as in AWS | nothing here would ever set `PENDING_ACTIVATION` or `PENDING_CLOSURE` |
| **B. Three: working, blocked, waiting for deletion** | each one is set by something | — |

**How the database keeps a state:**

| Option | Pros | Cons |
|---|---|---|
| **A. A type of PostgreSQL, `CREATE TYPE ... AS ENUM`** | the database refuses a misspelled word | the list a second time, in the database; a value cannot be removed without a new type |
| **B. `text`, and the list in the `State` enum in Rust** | one list | a misspelled word written by hand in SQL goes in |

**What the database keeps for a state that waits for deletion:**

| Option | Pros | Cons |
|---|---|---|
| **A. `deletes_at`, the date of deletion** | fixed when the deletion is asked for, so a later change of the window moves nobody's date | — |
| **B. `closed_at`, and the loop that deletes adds the window** | the window can change for everyone at once | after the window is shortened, somebody is deleted earlier than promised |

## Decision

**What a user has: A. The users of a deleted tenant: B. The links: B. The states: B. How they are kept: B. The date: A** (decided by the maintainer on 2026-11-20 and 2026-11-21).

- **`zgm_iam.users` has six columns:** `id uuid PRIMARY KEY DEFAULT uuidv7()`, `tenant_id uuid NOT NULL REFERENCES zgm_iam.tenants (id) ON DELETE CASCADE`, `state text NOT NULL DEFAULT 'active'`, `deletes_at timestamptz`, and `created_at` and `updated_at`, as in `tenants`. The trigger `touch_users` sets `updated_at` on every change. The index `users_tenant_listing_idx` on `(tenant_id, id)` serves both the foreign key and the list of users, and `users_tenant_state_listing_idx` on `(tenant_id, state, id)` the list of users in one state ([ADR-0010](0010-list-users.md)).
- **`zgm_iam.tenants` gets the same `state` and `deletes_at`.**
- **The states are `active`, `suspended` and `pending_deletion`.** Every new tenant and user is `active`. `suspended` is blocked, and can be unblocked. `pending_deletion` waits for deletion: it can be restored until `deletes_at`, and then the row is deleted. The first and the second have their names from AWS (the maintainer: "then make the same states as in AWS"). The third was first `CLOSED`, as in AWS; the maintainer did not like it ("name it differently somehow") and chose `pending_deletion`.
- **One state blocks, whoever blocks:** the maintainer: "I guess yes, one state". Who may block and unblock is a matter of permissions.
- **`deletes_at` is set together with `pending_deletion`** and is `NULL` in the other states. The code keeps them in step; the database does not check it.
- **The word of a state is plain `text`,** with a `--` comment in the migration that names the three and the `State` enum. `State` derives `strum::EnumString` with `serialize_all = "snake_case"`, as `zgm-limits` names its enums, so `str::parse` reads the word; a word that is no state is the error `IamError::UnknownState`, never taken for another state.
- **`Tenant` gives back `state: State` and `deletes_at: Option<Timestamp>`,** read from the row: `create_tenant` gives `State::Active` and `None`.
- **No link between rows leaves its tenant, and the code keeps it so.** A function that links two rows uses `INSERT ... SELECT ... WHERE` and looks for both in the tenant; no row written means an error. Every such function has a test in which the second row is of another tenant.

## Why

- **No name:** the maintainer: "if signing in is done by another module, then this name is probably not even needed". In AWS the name is where the user signs in and what the user is called in policies; here the login module does the first, and policies will point at a user by the UUID. A name can come back to somebody else: that is why AWS gives every user a unique ID next to the name ("even if you create a new IAM user that reuses a friendly name you deleted before"), and Cedar asks for "unique values that can't be reused".
- **`CASCADE`:** the window is the guard against a mistake: a tenant is deleted only after its window, while it can still be restored. A real `DELETE`, not a soft one, so that the cascade works.
- **The links in the code:** the maintainer: "let the database not check it artificially, a check in the code is enough".
- **Three states:** `PENDING_ACTIVATION` belongs to the sign-up, which the application and the login module do; this module creates a tenant when the sign-up is over. `PENDING_CLOSURE` is the time AWS needs to close an account in the background; here asking for a deletion is one write.
- **`text`:** the maintainer: "a comment in the code saying what the values are is enough, tests at the level of the code should catch it", and "only our code should operate on the tables, and no other". A column only one program writes keeps its list of values in that program, as `CHECK` constraints are left out for the same reason ([ADR-0002](0002-the-tenants-table.md)).
- **`deletes_at`:** the maintainer: "all right, then A".
- **`strum` 0.28.0 is the version `zgm-limits` uses.** Checked on 2026-11-21: it is the newest version, not yanked, OSV knows no vulnerabilities in it, and its repository is not archived. `Cargo.lock` had it already.

## Consequences

- **The module knows nothing about a user but its tenant, its state and its dates,** so a copy of the database names nobody ([ADR-0001](0001-nothing-says-whose-uuid-it-is.md)).
- **`create_user` writes a user and `user_id(id).delete()` puts it in `pending_deletion` with its `deletes_at`** ([ADR-0008](0008-the-functions-of-one-tenant-and-create-user.md), [ADR-0012](0012-delete-user.md)). `user_id(id).set_state(...)` blocks, unblocks and brings back a user waiting for deletion, emptying `deletes_at` ([ADR-0013](0013-the-functions-of-one-user-and-set-state.md)). The loop of the module deletes the users past their date ([ADR-0014](0014-the-loop-that-deletes-users.md)). The tests check the table itself with SQL written by hand.
- **A `DELETE` written by hand removes a tenant with all its users at once,** past any window. Only somebody who can write to the database can do it.
- **The platform cannot block one user against the will of the tenant:** the tenant can unblock its user. The platform blocks the whole tenant instead.
- **A word written by hand into `state` makes the next read fail.** For `create_tenant`, whose new row gets its state from the column's default, the tenant is then already written.
- **Nothing checks that every link stays inside its tenant but the code and its tests.**
