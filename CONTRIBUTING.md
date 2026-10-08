# Contributing

The project is an evolving native terminal IDE. Start with the usage guide and completion checklist, then choose one concrete behavior to improve.

Use the toolchain pinned in `rust-toolchain.toml`. The full test suite also needs Python 3 and Node 24 for protocol/extension fixtures; building and using the native editor requires neither runtime. Run:

```sh
node --test extension-host/api-types.test.cjs extension-host/api.test.cjs extension-host/host.test.cjs
cargo fmt --all --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --locked
python3 tests/pty_smoke.py target/debug/vscli  # Unix
```

Keep each commit atomic: one coherent change, its necessary tests, and relevant documentation. Every commit must build and keep the relevant checks passing. Do not split a feature across commits that leave unresolved imports or broken behavior.

Commit messages must be one line, at most 100 characters, using Conventional Commit prefixes, for example `feat: add snippet tab stops`, `fix: preserve edits after an external rename`, or `ci: test macOS terminal restoration`. Optional scopes are supported. Do not add a body, merge commit, or unrelated cleanup.

Open a feature branch and a pull request. Describe the observable change, test evidence, and remaining limitations. Main requires passing checks and linear history, including for administrators. Rebase merge preserves the reviewed atomic commits; merge commits and squash merging are disabled. Resolve review threads before merging. Dependency bot commits must be normalized to the same one-line policy before merging.

Tests should exercise behavior, not restate implementation. Changes to editing or persistence need data-integrity coverage. Terminal changes need a PTY scenario when practical. Compatibility claims need a named reference, platform/terminal configuration, and observable workflow evidence. Do not mark the project complete merely because a new intermediate feature works.

Release workflow dispatch performs a packaging rehearsal without publishing. To release, update the package version, lockfile, release notes, and compatibility documentation in reviewed commits. After main is green, push the matching `vVERSION` tag. CI runs again; only successful three-platform packages are published. Versions below 1.0 and versions with a prerelease suffix are marked prereleases. Published assets are never overwritten automatically.
