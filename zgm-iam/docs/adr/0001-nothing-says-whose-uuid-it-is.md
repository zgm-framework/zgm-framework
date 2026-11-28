# ADR-0001: The module's tables hold UUIDs in the open and nothing that says whose UUID it is

> **Status:** accepted
> **Date:** 2026-11-15
> **Applies to:** `zgm-iam` (every table of the module)

## Context

- **The module keeps the tenants of an application and what hangs under them.** Tables of this kind usually hold names and e-mail addresses in the open, so a copy of the database (a backup, a dump, a replica) lists the application's customers and their people.
- **The maintainer wants a copy of the module's database to be useless on its own:** "I want to be able to publish the database, and it would still give nobody anything." `zgm-limits` already works this way: its counters hold UUIDs and keyed hashes.
- **AWS IAM, the model for this module, keeps names readable to the service** and tells its users to keep sensitive data out of them: "We strongly recommend that you never put confidential or sensitive information, such as your customers' email addresses, into tags or free-form text fields such as a Name field."

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **A. Names and addresses in the open,** protected by access rights and disk encryption | search and sorting by name in SQL; nothing to build | a copy of the database names every tenant and every person |
| **B. UUIDs in the open; whatever links a UUID to a person or a company is a keyed hash, ciphertext, or not in the module at all** | a copy names nobody; identifiers stay plain columns, so keys, references and indexes work as usual | no search or sorting by name in SQL; the application needs keys; a copy still shows how many rows there are and when they were written |
| **C. As B, and the tenant's UUID itself kept as a keyed hash** | a copy cannot be matched even against a tenant UUID known from elsewhere | every call computes a keyed hash before it can find a row; it hides a value that names nobody; `zgm-limits` keeps the same UUID in the open |

## Decision

**B** (decided by the maintainer on 2026-11-15).

- **The UUID of a tenant, and of everything that hangs under a tenant, is a plain `uuid` column.**
- **A value that links a UUID to a person or a company** (a name, an e-mail address) is in the module's tables only as a keyed hash or as ciphertext, or it is not in the module at all and stays in the application.
- **The module computes neither.** The hash or the cipher, and their keys, belong to the application. The name of a tenant is the first such value ([ADR-0002](0002-the-tenants-table.md)).
- **The rule holds for every table the module gets later.**

## Why

- **What matters is the link, not the identifier.** The maintainer, on keeping the tenant's UUID in the open: "some UUIDs lie there, but nothing can be linked to them, and that is the point: the boundary of the link." This is why C adds a cost and no protection.
- **The maintainer chose the rule over the model:** a readable name "as in AWS" was the first choice, and was dropped once a leaked copy of the database was considered.

## Consequences

- **A copy of the database still shows how many and when:** how many tenants there are and when each row was created and changed. A UUID v7 carries its time of creation too.
- **Somebody who knows a UUID from elsewhere,** e.g. from a URL or a log, finds its rows, and learns the same: counts and dates.
- **The length of a ciphertext follows the length of what was encrypted,** unless the application's cipher pads it.
- **The rule protects a copy of the database, not the running system.** Whoever has the application's keys as well can read everything.
- **The rule is not a claim about data-protection law.** For the application, which can link a UUID to a person, the rows remain data about that person.
- **Names cannot be searched or sorted in SQL.**
