---
name: sources-release
description: Pins and activates external Git source inputs with Sources. Use when users request fixed dependency revisions, release preparation, or reproducible builds in a Sources project; propose Sources setup first if external Git checkouts are unmanaged. Excludes ordinary package-version updates without external source checkouts.
---

# Pin and release source inputs

Require Git, an installed `sources` CLI, and a configured `.sources.conf`; see the [official README](https://github.com/refinio/sources#readme) for installation and setup. If adoption is needed, ask whether the user wants Sources before changing the project.

Work from the app root or use `sources -C <app-root> ...`. Establish whether the user wants new pins or reproduction of existing pins. Release activation never repins dependencies.

To capture a newly tested development combination, first commit and make source changes available upstream within the user's authorized scope. Dirty development edits are excluded from pins.

```sh
sources check
sources lock --check
git diff -- .sources.lock
```

Unqualified `sources lock` captures every configured development checkout's HEAD, regardless of active mode, without fetching. Adding `--check` ensures proposed commits and recursive submodule inputs are cached from configured upstreams, fetching missing inputs; cache presence does not prove current remote availability or test success.

For a partial update, replace the example commit with the intended full commit ID:

```sh
sources lock lib:<full-commit> --check
```

Review the complete `.sources.lock`; commit approved pins with the app changes when authorized. For reproduction, retain the lock belonging to the selected app revision and do not run `lock` merely to activate it.

Use a clean app checkout at the intended revision for an online release build:

```sh
sources release --clean
sources status
git rev-parse HEAD
```

`--clean` discards all managed checkout contents, including edits, untracked files, dependency installations, and ignored outputs, then recreates pinned parent and recursive submodule checkouts from upstream. Preserve valuable managed edits elsewhere before using it. Original development checkouts are untouched.

For local switching, `sources release` retains nonconflicting ignored artifacts, obtains missing inputs, and refuses dirty managed sources or overwrite conflicts. Retained outputs may need rebuilding.

For offline reproduction, use `sources release --offline` only with complete cached parent and recursive submodule commits, trees, and file contents. It makes no network requests and cannot be combined with `--clean`. `sources fetch` or `sources check-release` can prepare caches online without activating or repinning.

Install dependencies using the project's frozen package lock, then build and test with its existing tooling. Sources does not clean the app's outputs; use a fresh app checkout or its established cleanup procedure separately. Record the app HEAD, source lock, tooling, and build/test results with release evidence.

Report failures and the suggested repair command; rerun activation after fixing the cause. Return to development links with `sources dev` when requested. Do not publish artifacts or create a release unless authorized. See the [README](https://github.com/refinio/sources#readme) for details.
