# zgm-framework

Building blocks for multi-tenant applications in Rust, on PostgreSQL.

This is a collection of modules extracted from [leads.run](https://leads.run), which uses them. Each module is a separate crate that can be used on its own.

| Module | What it does |
|---|---|
| [`zgm-limits`](zgm-limits/README.md) | Attempt limits (sign-in, codes, exports) counted in PostgreSQL: one statement per attempt, rules a tenant can override, a list of blocked keys, audit events. Depends on no other module. |

More modules will be added here as they are ready.

## Using a module

Every module has its own version and its own tags, named `<module>-v<version>`. Depend on the one you need; Cargo builds only that crate.

```toml
[dependencies]
zgm-limits = { git = "https://github.com/zgm-framework/zgm-framework", tag = "zgm-limits-v0.1.0" }
```

Each module's README says what it needs and how to use it, and each keeps a record of its design decisions in its `docs/adr` folder.

## Development

The repository is one Cargo workspace. The tests need an empty PostgreSQL 18 database in `DATABASE_URL`.

```bash
docker run -d --name zgm-postgres -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:18
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres

cargo test --workspace
```

## Author

Built and maintained by Paweł Zygmunt ([LinkedIn](https://www.linkedin.com/in/pawel-zygmunt-226831181/), [GitHub](https://github.com/Pawel-Zygmunt)), who also builds [leads.run](https://leads.run). Every design decision is written down next to the code, with the options that were rejected and the measurements behind the choice.

## Bugs, questions and vulnerabilities

- **Bugs, questions and ideas:** open an [issue](https://github.com/zgm-framework/zgm-framework/issues) and say which module it is about.
- **Vulnerabilities:** report them privately, so that a fix can be released before the details are public; see [SECURITY.md](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this repository by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
