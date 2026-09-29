# Contributing to Wise Bucket

Thank you for your interest! Wise Bucket is built **one milestone at a time**; each milestone is small, tested and documented.

## Ground rules

- Be kind: see the [Code of Conduct](CODE_OF_CONDUCT.md).
- For anything bigger than a small fix, **open an issue first** so we can agree on the approach.
- **Documentation is part of the change.** A pull request that changes behaviour updates `docs/guide.md` or `docs/troubleshooting.md` (troubleshooting and FAQ) when useful, and `CHANGELOG.md`.

## Development setup

Requirements: Rust (the pinned toolchain in `rust-toolchain.toml` is installed automatically by rustup), a C compiler and `make` (used to build pgvector for tests), and optionally Docker (to test the bring-your-own-PostgreSQL mode).

```sh
cargo build
cargo test --workspace
```

The first test run downloads PostgreSQL and builds pgvector into `target/tmp/wb-runtime` (about 1 minute); later runs reuse it. Set `WB_TEST_RUNTIME_DIR` to share it between checkouts.

To also run the bring-your-own-PostgreSQL test:

```sh
docker run -d --rm --name wb-pg -e POSTGRES_PASSWORD=pw -p 127.0.0.1:55432:5432 pgvector/pgvector:pg17
WB_TEST_DATABASE_URL=postgres://postgres:pw@127.0.0.1:55432/postgres cargo test --workspace
```

## Before opening a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p wb-server -- docs gen       # regenerate docs/reference/*
```

CI runs the same checks on Linux and macOS, plus `cargo deny`, link checking and spell checking.

## Conventions

- **Errors:** every user-facing error is a `WbError` with a stable code from `crates/wb-core/src/error.rs` and a documented fix. Adding a code means adding its title and fix there; `docs gen` renders `docs/reference/errors.md`, and a test fails if a code is undocumented.
- **Configuration keys:** documented in `KEYS` in `crates/wb-core/src/config.rs`, rendered to `docs/reference/configuration.md`.
- **Output:** human messages go to stderr; stdout is for data (`--json`) and, in `serve`, reserved for MCP messages.
- **No LLM calls, no network at runtime:** only `setup` downloads, always checksum-verified.
- **Tests:** acceptance tests for a milestone live in `crates/wb-server/tests/` and drive the real binary with a real MCP client.
- **Commits:** clear, imperative messages ("Add roots check command"). Sign off your commits (`git commit -s`) to certify the [Developer Certificate of Origin](https://developercertificate.org).

## Labels

| Label | Meaning |
| --- | --- |
| `good first issue` | Small, well-scoped, mentored |
| `bug` / `enhancement` / `docs` | Kind of change |
| `milestone:Mx` | Planned for milestone x |
| `needs-design` | Discuss before coding |

## License

By contributing, you agree that your contributions are licensed under the [Apache License 2.0](LICENSE).
