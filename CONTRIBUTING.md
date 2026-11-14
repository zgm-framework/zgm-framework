# Contributing

Thanks for taking the time. Feedback of every size is welcome: a question, a bug report, a fix, a new idea.

## Before you write code

- **A small fix** (a typo, a clear bug, a missing test): open a pull request straight away.
- **Anything larger** (a new feature, a change of behavior, a new dependency): open an issue first and describe what you need and why. It saves you from writing something that does not fit the module.
- **A vulnerability:** do not open an issue or a pull request. Report it privately; see [SECURITY.md](SECURITY.md).

## Running it

You need Rust (the version is pinned in `rust-toolchain.toml` and installed by `rustup` on first use) and an empty PostgreSQL 18 database.

```bash
docker run -d --name zgm-postgres -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:18
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres

cargo test --workspace                       # every test creates its own empty database
cargo run -p zgm-limits --example tour       # a run through every feature of zgm-limits
```

## What CI checks

A pull request must pass the same three commands CI runs:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## How the code is written

These rules hold in every module. Each has a decision record that explains why.

- **A bug fix comes with a test that fails without it.** Tests live in the module's `tests/` folder, go through the public API, and run on a real database.
- **All SQL of a module lives in its `src/repository.rs`,** in plain functions that take a connection. Methods with logic hold no SQL.
- **Errors are enums derived with `thiserror`.**
- **Dependencies are pinned to an exact version** (`=x.y.z`). A new dependency needs a reason in the pull request.
- **A module gives mechanisms, not values from standards.** It enforces only what makes the mechanism work at all (a positive number, a type); ranges that a regulation asks for are set by whoever configures the system.
- **A choice between solutions gets a decision record** in the module's `docs/adr` folder: the context, the options with their costs, the decision and why. A record describes the decision in force today, so changing a decision means rewriting its record.
- **Everything is in English:** code, comments, documents and commit messages.

## License

By contributing you agree that your contribution is dual licensed under MIT and Apache-2.0, like the rest of the repository.
