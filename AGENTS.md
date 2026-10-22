# Sources CLI

Read `docs/sources-cli-spec.md` before changing behavior. This is an experimental Rust CLI that delegates Git operations to the installed Git executable. Linux and macOS are required; Windows is best effort.

Use shallow fetching with complete checkouts and recursive submodules. Managed `.sources/` state is disposable. Do not add rollback machinery or independent remote-availability proofs.

The agreed acceptance-test seam is the public CLI, using real temporary repositories, filesystem links, and child processes. Configuration and identity normalization may also be tested through their public module interfaces. Follow test-first vertical slices and keep tests with the behavior they prove.

Keep the suite limited to critical public API unit tests and one high-level integration scenario per main user story. Extend the relevant story for a regression rather than adding a test for each branch or variation. Do not restore exhaustive transport, filename, shell or malformed-input matrices. See `docs/testing.md` for the retained coverage and verification commands.

Run `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, and `cargo test --locked` before delivery. Every commit is reviewed asynchronously by roborev; resolve relevant reviews and run the whole-change review before declaring completion.

## Repository-local skills and refactoring

- When the same skill name is available from multiple locations, always use the repository-local skill under `$REPO_ROOT/.agents/skills/`. Repository-local skills take precedence over user-global skills. Do not load or apply the corresponding skill from `$HOME/.agents/skills/` when a repository-local version exists.
- Consider the repository's current publication state and backwards-compatibility requirements recorded in `.agents/refactor-policy.md` in all interactions.
