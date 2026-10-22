# Test scope

Maintain critical public API unit tests and one integration scenario per main user story. Prefer extending an existing story over adding another fixture for a regression. Keep assertions about observable behavior; do not assert private call order or source text. A scenario may cover success, the user-visible failure and recovery in the same workflow.

The suite deliberately retires exhaustive malformed-input, transport fallback, unusual filename, shell job-control and repeated configuration/selection matrices. These remain supported behaviors where specified; there is no longer a permanent test for every variation. Runtime code and the public CLI are unchanged by this reduction.

## Critical API tests

`tests/api.rs` calls public modules directly without CLI, Git, pnpm or Turbo subprocesses. Four tests cover repository identity boundaries, Turbo environment-name normalization, configuration path and package ownership boundaries, and exact Git/registry pins. Temporary files are used only where the public configuration API reads persisted input.

## Main user stories

| Story | Scenario |
| --- | --- |
| Set up a project repeatedly without overwriting configuration | `tests/setup.rs` |
| Inspect a development checkout through a fork remote and discover its project | `tests/check.rs` |
| Capture development heads, switch to pinned Git inputs, and return to development | `tests/release.rs` |
| Discard disposable managed changes while preserving external development files | `tests/release.rs` |
| Reuse cached inputs offline and recover a failed clean release | `tests/release.rs` |
| Activate and update recorded recursive submodule pins | `tests/activation.rs` |
| Capture exact upstream release tags without changing development checkouts, and preserve pins on failure | `tests/pins.rs` |
| Fetch a shallow complete revision and inspect cached readiness offline | `tests/network.rs` |
| Inspect active/dirty/broken links and receive repair guidance | `tests/status.rs` |
| Switch real installed packages among development, pinned Git and registry selections | `tests/pnpm.rs` |
| Check registry pins by downloading archives without activating them, preserving pins on failure | `tests/pnpm.rs` |
| Run a child in the selected checkout with literal arguments, inherited I/O and its exit status | `tests/run.rs` |
| Terminate a child process tree while retaining the project lock through shutdown | `tests/run.rs` |
| Execute exact allowed package/task pairs and their Turbo prerequisites | `tests/turbo.rs` |
| Run concurrent nested prerequisites from parent Turbo and serialize child execution | `tests/turbo_parent.rs` |
| Propagate a leaf change through direct repository edges, hit cache unchanged and restore deleted outputs | `tests/turbo_transitive.rs` |
| Resolve only consumed task hashes and preserve cache across unrelated task changes | `tests/turbo_transitive.rs` |
| Build and restore a repository cycle with an acyclic foundation task graph | `tests/turbo_transitive.rs` |
| Reject a genuine task-hash cycle before execution with an actionable task path | `tests/turbo_transitive.rs` |
| Execute a single-package repository with colon-containing task names | `tests/turbo_single.rs` |
| Use native Windows paths for recursive release and child execution | `tests/windows.rs`, Windows only |

Two `run.rs` test-harness entry points launch real fixture child processes. They are helpers for the shutdown story, not additional user stories. Keep those entry points with their caller.

## Verification

Use the pinned Turbo executable installed by CI. For example, after installing `turbo@2.7.6` into a task-owned tools directory on Linux:

```sh
export SOURCES_TEST_TURBO=/path/to/tools/node_modules/turbo-linux-64/bin/turbo
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

Use the matching native Turbo binary on macOS or Windows. The CLI stories also require Git, Node, Python 3 and pnpm 10.15.0. Fixtures isolate Git configuration and serve archives locally.

Distribution has a separate user story: install a native archive and run the packaged CLI. Its existing release gates in `.github/scripts/` verify matching versions, archive checksums, package smoke behavior and generated installer success/failure. Run those when changing packaging or installers; Rust checks alone do not cover installer behavior. This reduction does not remove native publication gates.
