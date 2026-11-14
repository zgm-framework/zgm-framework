# ADR-0013: The hash of the hidden part is computed by a `KeyedHash` from the application, required in the builder

> **Status:** accepted
> **Date:** 2026-11-10
> **Applies to:** `zgm-limits` (`src/keyed_hash.rs`, `Limits::builder`, `Limits::hit`, the `key_hash` column)

## Context

- **The hidden part of a key (an e-mail, an IP address) lies in the database only as a hash** ([ADR-0005](0005-counters-table-and-key.md)). Until now the caller gave a ready list of hashes.
- **The module has no cryptography and no keys of its own.** The secret key and its versions belong to the application ([ADR-0008](0008-changing-the-secret-key.md)).

## Options considered

**The name:**

| Option | Pros | Cons |
|---|---|---|
| **A. "Fingerprint": `Fingerprinter`, `key_fingerprint`** | as in `zgm-encryption` | less technical |
| **B. "Hash": `KeyedHash`, `key_hash`** | a technical name; "keyed" says that a secret key is needed | a different name than in `zgm-encryption`; `Hash` and `Hasher` are already in the standard library, so the trait cannot have the same name |

**Whether the method is `async`:**

| Option | Pros | Cons |
|---|---|---|
| **A. A plain method, with no error** | a simpler trait, no crate | the implementation must already have the keys in memory; it cannot fetch them on the way and has no way to report an error |
| **B. `async` and `Result`** | the implementation can fetch the key from anywhere and say that it failed | the `async-trait` crate; every call allocates a small piece of memory for the future |

**Whether the builder requires an implementation:**

| Option | Pros | Cons |
|---|---|---|
| **A. Required** | a module in which `.hidden(...)` does not work cannot be built; no "no hash was given" error at run time | an application with no hidden parts must give something too |
| **B. Optional** | an application with no hidden parts gives nothing | `.hidden(...)` without an implementation fails only at the attempt |

## Decision

**The name: B, everywhere. `async`: B. The builder: A** (decided by the maintainer on 2026-11-10).

- **The `KeyedHash` trait has one method, `async fn hashes(data)`.** It returns a hash for every version of the secret key that can still be read, the current one first, or the error `KeyedHashError`, which wraps the implementation's error.
- **An error of the implementation is the error of the attempt:** `limits.attempts(...)` returns `LimitsError::KeyedHash`, and the attempt is not counted. The limit is not skipped silently.
- **The trait is marked `#[async_trait]`** from the `async-trait` crate 0.1.92, the same version as in four other crates of the repository.
- **The builder requires `.keyed_hash(...)`,** also when no limit of the application has a hidden part.
- **The hidden part is given with `.hidden(&email)`** when the `Attempts` object is built ([ADR-0014](0014-attempts-object-hit-reset.md)). The temporary list of ready hashes is gone.
- **`data` is put together by the module:** the limit name, the tenant, the account and the hidden part, every piece with its length in 8 bytes in front of it.
- **`.hidden(...)` makes the text uniform:** it turns letters to lower case and removes all white space and invisible characters, also from the middle. `Anna@Firma.pl `, `anna @firma.pl` and `anna@firma.pl` are one key (decided by the maintainer on 2026-11-10: "the plain hidden should lower-case the letters, remove spaces, trim").
- **Invisible characters are the control and formatting characters of Unicode,** e.g. the zero-width space (U+200B), the byte order mark (U+FEFF), the soft hyphen (U+00AD) and the word joiner (U+2060). Pasted into the middle of an address they do not change how it looks, so without removing them one address would get any number of counters; a code review showed this. The list of blocks is in `keyed_hash::is_invisible`.
- **`.hidden_exact(...)` takes the text unchanged,** e.g. for an identifier in which letter case matters.
- **Both at once is the error `LimitsError::HiddenAndHiddenExact`,** and the attempt is not counted.
- **An empty list or an empty hash is the error `LimitsError::KeyedHashGaveNoHash`,** and the attempt is not counted.
- **A hash longer than 128 bytes (`MAX_HASH_BYTES`) is the error `LimitsError::KeyedHashGaveLongHash`,** and the attempt is not counted. HMAC-SHA-512 gives 64 bytes, and a header with the version of the key a few more, so 128 leaves a margin.
- **A key without a hidden part does not call `KeyedHash` at all.**
- **The word "hash" replaced "fingerprint" in the code, the migration and the documents:** the `key_hash` column, the `Blocked.key_hash` field.
- **`Limits` does not show `KeyedHash` in `Debug`,** because the implementation holds secret keys.

## Why

- **The lengths in front of the pieces** make sure that two different keys do not give the same bytes: without them, "ab" + "c" and "a" + "bc" would be the same.
- **An empty hash means "a key without a hidden part" in the table,** so an implementation that returned one would merge two different keys into one counter.
- **The maintainer foresees an implementation that fetches the key** ("and what if I needed my hasher to be async, because it fetches the key from somewhere?"). Fetching can fail, so `async` alone, without `Result`, would not be enough.
- **`async-trait` checked on 2026-11-10:** the version is not yanked, OSV knows no vulnerabilities in it, and the repository is not archived.
- **Without making the text uniform, a change of letter case would get around a limit on an e-mail.**
- **With both at once, the module does not guess which text is the key,** because it would silently count on a counter the caller did not mean.
- **The hash lies in the primary key of the table,** and PostgreSQL refuses an index entry longer than about 2700 bytes. The bound in the module turns a database error at the write into a clear error when the object is built.
- **A required implementation means fewer states:** the module always has a hash, not "has one or does not".

## Consequences

- **An implementation with the key in memory writes an `async fn` that awaits nothing.** It costs one small memory allocation per call, which does not matter next to the database query that follows right after.
- **An application that implements the trait needs the `async-trait` crate too.**
- **A plain hash without a secret key, e.g. SHA-256 alone, compiles too.** A copy of the database then lets anybody guess the IP addresses in a few minutes. The trait says so in its documentation; the code cannot check it.
- **An implementation that returns `data` unchanged does not work,** because the data is usually longer than 128 bytes. It used to be allowed for an application that has nothing to hide; a code review showed that a text of 3000 bytes in the "e-mail" field then ended every `hit` with the database error "index row size exceeds maximum", and the attempt was never counted. The example in `examples/tour/example.rs` has `ExampleHash`: a hash from the standard library, without a key, described as not for a real application.
- **The recommended implementation is `zgm_encryption::Fingerprinter`** (HMAC-SHA-256). Its hash starts with a format byte and the number of the key version, which the future method "how many rows each key version has" will use ([ADR-0005](0005-counters-table-and-key.md)). The module itself does not require this header.
- **`hidden` removes white space from the middle of the text too,** not only from the ends. "Jan Kowalski" and "jankowalski" are one key here.
- **`hidden` does not understand e-mail addresses:** `anna+1@firma.pl` and `anna+2@firma.pl` are two keys. The caller canonicalizes such an address.
- **`hidden` does not make uniform the different ways of writing the same letter in Unicode,** e.g. `é` as one character and as `e` with an accent, nor full-width letters. That would need Unicode tables from a separate crate.
- **The list of invisible characters is a set of chosen blocks, not the whole "format" category of Unicode,** for the same reason.
- **A text that is already uniform gives the same key through `hidden` and through `hidden_exact`.**
- **There is one hidden part.**
