---
name: sources-release
description: Pins and selects external Git sources or configured pnpm package releases with Sources. Use when users request fixed dependency revisions, release preparation, or reproducible builds in a Sources project; propose Sources setup first if external Git checkouts are unmanaged. Excludes ordinary package-version updates without external source checkouts.
---

# Pin and release source inputs

Require Git, an installed `sources` CLI, and a configured `.sources.conf`; see the [official README](https://github.com/refinio/sources#readme) for installation and setup. If adoption is needed, ask whether the user wants Sources before changing the project.

Work from the app root or use `sources -C <app-root> ...`. Establish whether the user wants new pins or reproduction of existing pins. Release activation never changes Git pins or configured exact package versions. Opt-in pnpm switching changes effective overrides and may update its dependency lock.

To capture a newly tested development combination, first commit and make source changes available upstream within the user's authorized scope. Dirty development edits are excluded from pins.

```sh
sources check
sources lock --check
git diff -- .sources.lock
```

Unqualified `sources lock` captures every Git-release development checkout's HEAD, regardless of active mode, without fetching. Adding `--check` ensures proposed commits and recursive submodule inputs are cached from configured upstreams, fetching missing inputs; cache presence does not prove current remote availability or test success.

For a partial update, replace the example commit with the intended full commit ID:

```sh
sources lock lib:<full-commit> --check
```

For a Git tag, use `sources lock lib:tag=v1.2.3 --check`; it stores the resolved commit, not a moving tag.

For sources with `pnpm` package mappings, select exact published versions in `.sources.conf`. No Git pin is required for those sources. `sources dev` selects file overrides; `sources release` selects registry versions. Both run pnpm install, may access the registry, and may update `pnpm-lock.yaml`. Review and commit the release-mode workspace overrides, pnpm lock, and configuration together. `lock`, `fetch`, and `check-release` handle Git inputs only.

Review the complete `.sources.lock`; commit approved pins with the app changes when authorized. For reproduction, retain the lock belonging to the selected app revision and do not run `lock` merely to activate it.

Use a clean app checkout at the intended revision for an online release build:

```sh
sources release --clean
sources status
git rev-parse HEAD
```

`--clean` discards all managed checkout contents, including edits, untracked files, dependency installations, and ignored outputs, then recreates pinned parent and recursive submodule checkouts from upstream. Preserve valuable managed edits elsewhere before using it. Original development checkouts are untouched.

For local switching, `sources release` retains nonconflicting ignored artifacts, obtains missing inputs, and refuses dirty managed sources or overwrite conflicts. Retained outputs may need rebuilding.

For offline reproduction, use `sources release --offline` only with complete cached parent and recursive submodule commits, trees, and file contents. It forwards `--offline` to pnpm when enabled, requiring its package inputs in the local store too. It cannot be combined with `--clean`. `sources fetch` or `sources check-release` can prepare caches online without activating or repinning.

For pnpm integration, release already installs dependencies with lock updates allowed; verify the committed release state with `pnpm install --frozen-lockfile`. Otherwise install with the project's frozen package lock. Build and test with existing tooling. Sources does not clean the app's outputs; use a fresh app checkout or its established cleanup procedure separately. Record the app HEAD, source lock, tooling, and build/test results with release evidence.

Report failures and the suggested repair command; rerun activation after fixing the cause. Return to development links with `sources dev` when requested. Do not publish artifacts or create a release unless authorized. See the [README](https://github.com/refinio/sources#readme) for details.
