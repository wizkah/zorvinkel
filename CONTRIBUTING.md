# Contributing

Thanks for your interest in Zorvinkel. The project is in early development (v0.1, pre-release), so the design and the code still change quickly.

## Before you start

- For anything larger than a small fix, please open an issue first so we can agree on the approach.
- Design decisions are recorded as ADRs in [`docs/architecture/adr/`](docs/architecture/adr/). Read the relevant ones before changing behavior, and add a new ADR for significant decisions.
- Security vulnerabilities do not belong in public issues. See [SECURITY.md](SECURITY.md).

## Building and testing

The CI runs these checks; please run them locally before opening a pull request:

```sh
cargo fmt --all -- --check
cargo build --workspace --all-targets --locked
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

On Linux you need `libdbus-1-dev` and `pkg-config`. The client, `zorvinkeld` and the secure channel can be tried without hardware using `--simulate` (see the CI workflow for the exact commands). Build and packaging notes are in [`packaging/BUILD.md`](packaging/BUILD.md).

## Code style

Follow the guidelines in [`docs/development/Clean-Code.md`](docs/development/Clean-Code.md). `unsafe` code is denied in the workspace. Update `CHANGELOG.md` for user-visible changes.

## License

By contributing you agree that your contribution is licensed under GPL-3.0-or-later, like the rest of the project.
