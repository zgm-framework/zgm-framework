# ADR-0004: What the module keeps readable nowhere is encrypted by a `Cipher` from the application, required in the builder

> **Status:** accepted
> **Date:** 2026-11-20
> **Applies to:** `zgm-iam` (`src/cipher.rs`, `Iam::builder`, `Iam::create_tenant`, the `name` column of `zgm_iam.tenants`)

## Context

- **The name of a tenant lies in the table only as ciphertext** ([ADR-0002](0002-the-tenants-table.md)), and the module must be able to show it, so a hash is not enough.
- **The module has no cryptography and no keys of its own.** The maintainer: "this IAM module should not deal with encryption", "the encrypting and decrypting method should be provided by a trait".
- **`zgm-limits` solved the same for hashes** with a `KeyedHash` trait the application implements and gives to the builder.

## Options considered

**What the methods get:**

| Option | Pros | Cons |
|---|---|---|
| **A. Only the bytes** | one trait for everything the module will encrypt; the trait knows nothing of tenants, columns or rows | the implementation cannot tell tenants apart, so it has one key for all; it cannot bind a ciphertext to its row |
| **B. The bytes and their place** (the tenant, the column) | a key per tenant is possible; a ciphertext copied to another row does not decrypt there | the trait has to name every place the module encrypts something, and changes with each new one |

**Whether the methods are `async`:**

| Option | Pros | Cons |
|---|---|---|
| **A. Plain methods, with no error** | a simpler trait, no crate | the implementation must already have its key in memory; it cannot fetch it on the way and has no way to report an error |
| **B. `async` and `Result`, as `KeyedHash` in `zgm-limits`** | the implementation can fetch its key from anywhere and say that it failed | the `async-trait` crate; every call allocates a small piece of memory for the future |

**The name:**

| Option | Pros | Cons |
|---|---|---|
| **A. `Encryptor`** | says what it is for | names one of its two directions |
| **B. `Cipher`** | the usual name of something that does both | none found |

## Decision

**What the methods get: A. `async`: B. The name: B** (decided by the maintainer: the bytes only and the name on 2026-11-15, `async` and `Result` on 2026-11-20).

- **The `Cipher` trait has two methods,** `async fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, CipherError>` and `async fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, CipherError>`. It is marked `#[async_trait]` and requires `Send + Sync + 'static`.
- **`CipherError` wraps the implementation's own error,** the same way `KeyedHashError` does in `zgm-limits`.
- **The builder of `Iam` requires `.cipher(...)`** ([ADR-0005](0005-iam-with-a-bon-builder.md)), once for the whole module, and the module uses that one cipher for everything it encrypts (decided by the maintainer on 2026-11-15).
- **An error of `encrypt` is the error of the operation:** nothing is written, and the caller gets `IamError::Cipher`.
- **Empty bytes from `encrypt` are the error `IamError::EmptyCiphertext`,** and nothing is written.
- **`decrypt` is called by `set_state` and `delete` of a tenant,** which give the tenant back with its name, in the transaction of the change, so that a cipher that fails changes nothing ([ADR-0015](0015-the-state-of-a-tenant.md)).
- **`Iam` does not show the cipher in `Debug`,** because the implementation holds secret keys.

## Why

- **The bytes only:** the maintainer: "this encrypt/decrypt method does not know what comes in; it simply has a data type, and the module can use it underneath for different things; now it will encrypt the name of a tenant, but soon probably something else".
- **`async` and `Result`:** an application may keep its key in a key store and fetch it over the network, and the store may not answer.
- **Empty bytes:** a ciphertext is never empty, so they come only from a broken cipher. The column is `NOT NULL`, which empty bytes pass, and the tenant would have no name.
- **`async-trait` 0.1.92 is the version `zgm-limits` uses.** Checked on 2026-11-20: it is the newest version, not yanked, OSV knows no vulnerabilities in it, and its repository is not archived.

## Consequences

- **One key for all tenants.** The maintainer accepted it: "yes, well, one key".
- **An implementation that can bind a ciphertext to a place gets the bytes only,** so it binds everything to one fixed place. Bytes copied to another row then decrypt there too. That matters only against somebody who can write to the database.
- **The module knows no versions of the key.** A ciphertext is only decrypted, never compared, so the implementation can tell from the bytes which version made them ([ADR-0002](0002-the-tenants-table.md)).
- **An implementation that implements the trait needs the `async-trait` crate too.** One with the key in memory writes an `async fn` that awaits nothing.
- **A "cipher" that gives the bytes back unchanged, or only scrambles them, compiles and works.** The module cannot tell ciphertext from anything else, so whether a copy of the database names nobody depends on the application's implementation.
