# Architecture decision records of `zgm-limits`

Every choice of a concrete solution in this crate has a record here: the context, the options considered with their pros and cons, the decision, and why. A record describes the decision in force today; when a decision changes, its record is rewritten, so there is no trail of superseded records to read through.

A few things that help when reading them:

- **"The maintainer"** is the author of the crate, who makes the decisions.
- **Other crates by the same author** are mentioned here and there (`zgm-encryption`, `zgm-keys`, `zgm-audit`, `zgm-admin`, `zgm-iam`, `zgm-credentials`). None of them is needed to use this crate.
- **Measurements** were taken on one laptop with a local PostgreSQL 18. Ratios come from two variants alternated over at least five rounds; absolute numbers will differ elsewhere.

| No. | Decision | Status | Date |
|---|---|---|---|
| [0001](0001-commands-in-a-justfile.md) | The commands that build and run the crate live in a `justfile` | accepted | 2026-11-10 |
| [0002](0002-window-from-the-first-attempt-and-rule-as-an-enum.md) | A limit is counted with a window from the first attempt, and `Rule` is an enum with a variant for each way of counting | accepted | 2026-11-10 |
| [0003](0003-limit-names-and-list-through-strum.md) | Limits are an enum of the product, and `strum` gives their names and their list | accepted | 2026-11-10 |
| [0004](0004-bucket-and-escalating-block-postponed.md) | Counting a bucket and a rule with an escalating block are postponed; the window comes first | accepted | 2026-11-10 |
| [0005](0005-counters-table-and-key.md) | One counters table with a primary key of four columns without empty values; the counter's state in `jsonb`; a hash only with a hidden part | accepted | 2026-11-10 |
| [0006](0006-counting-in-one-sql-statement.md) | An attempt is counted by one SQL statement, which reads the row first and writes nothing on a refusal | accepted | 2026-11-10 |
| [0007](0007-database-through-sqlx.md) | We connect to the database through `sqlx`, and all SQL lives in one `repository.rs` | accepted | 2026-11-10 |
| [0008](0008-changing-the-secret-key.md) | After the secret key changes, we look for the counter with the hashes of all versions and rewrite nothing | accepted | 2026-11-10 |
| [0009](0009-limits-with-a-bon-builder.md) | The entry point of the module is the `Limits` struct, built with a `bon` builder | accepted | 2026-11-10 |
| [0010](0010-tests-in-the-tests-folder-on-a-database.md) | Tests live in the `tests/` folder and check the module from the outside, on a real database | accepted | 2026-11-10 |
| [0011](0011-clock-from-the-builder.md) | The moment of an attempt comes from the module's clock, which a test replaces in the builder | accepted | 2026-11-10 |
| [0012](0012-list-of-blocked-keys.md) | A row remembers in the `blocked_at` column since when the key has been blocked; `get_blocked` lists blocked keys in pages, and `unblock` deletes the counter | accepted | 2026-11-10 |
| [0013](0013-keyed-hash-from-the-application.md) | The hash of the hidden part is computed by a `KeyedHash` from the application, required in the builder | accepted | 2026-11-10 |
| [0014](0014-attempts-object-hit-reset.md) | The application builds an `Attempts` object once for a limit and a key, and it has `hit` and `reset`; the attempt is counted before the action | accepted | 2026-11-11 |
| [0015](0015-cleanup-in-a-job-loop.md) | Counters that have been dead for an hour are deleted by the module's loop, started by a required builder step | accepted | 2026-11-10 |
| [0016](0016-rules-checked-at-build.md) | The rule of a limit is made of the types `Count` and `Period`, which accept no bad values, and the builder reads the rule of every limit at startup | accepted | 2026-11-11 |
| [0017](0017-tenant-rules-in-the-database.md) | Rules in code are the defaults, a tenant overrides them in the `tenant_rules` table, and an open window is counted to its end by the rule it opened with | accepted | 2026-11-11 |
| [0018](0018-override-bounds-in-strum-attributes.md) | The product writes the bounds of what a tenant may change a rule to as four `strum` attributes above the limit; all of them or none | accepted | 2026-11-11 |
| [0019](0019-the-module-runs-its-migrations.md) | The `migrate` function creates the module's tables, and the migration history lives in the module's schema | accepted | 2026-11-11 |
| [0020](0020-recommended-pool-settings.md) | We recommend a pool that does not test a connection before use and has a short idle timeout | accepted | 2026-11-11 |
| [0021](0021-events-instead-of-logs.md) | The module does not log; it reports events to the `Audit` given by the application; the event of a change goes in the transaction of that change, and the event of an attempt goes after the fact | accepted | 2026-11-11 |
| [0022](0022-window-of-distinct-elements.md) | "At most N distinct" is a window filled by distinct elements, with the list of elements in the counter's row | accepted | 2026-11-11 |
| [0023](0023-the-database-bounds-waiting-for-a-row.md) | Waiting for a held row is bounded by a database setting, not by the module | accepted | 2026-11-11 |
