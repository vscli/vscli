# Repository workflow

- Keep the native Rust editor usable without a JavaScript runtime. Optional compatibility processes must remain isolated from core editing.
- Work on a feature branch and use pull requests after the initial bootstrap. Main is protected and accepts linear, rebased history after required checks pass.
- Make each commit one coherent, buildable change with its necessary tests. Use a one-line Conventional Commit message of at most 100 characters (`feat:`, `fix:`, `test:`, `docs:`, `ci:`, `chore:`, etc.). Do not add commit bodies or merge commits.
- Run relevant behavior tests, `cargo fmt --all --check`, and `cargo clippy --all-targets --locked -- -D warnings`. Editing/persistence changes need data-integrity tests; terminal interactions should have PTY coverage.
- Preserve unsaved work, shared document identity, and version checks on asynchronous replies. Bound background queues and keep blocking work out of rendering/input paths.
- Update usage and compatibility documentation when behavior changes. Distinguish implementation, test evidence, and outstanding qualification; do not claim full VS Code parity or a completed project at an intermediate milestone.
- Release workflow dispatch is a packaging rehearsal. Publishing requires an intentional matching version tag after reviewed version/release-note changes and passing checks.
