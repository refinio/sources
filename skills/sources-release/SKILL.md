---
name: sources-release
description: Pins and selects external Git sources or configured pnpm package releases with Sources. Use when users request fixed dependency revisions, release preparation, or reproducible builds in a Sources project; propose Sources setup first if external Git checkouts are unmanaged. Excludes ordinary package-version updates without external source checkouts.
---

# Pin and release source inputs

Require Git, an installed `sources` CLI, and a configured `.sources.conf`; see the [official README](https://github.com/refinio/sources#readme) for installation and setup. If adoption is needed, ask whether the user wants Sources before changing the project.

Work from the app root or use `sources -C <app-root> ...`. Establish whether the user wants new pins or reproduction of existing pins. Release activation never changes Git pins or locked package versions. Opt-in pnpm switching changes effective overrides and may update its dependency lock.

Per-source `dev_hint` values in `.sources.conf` are advisory Git refs for development setup and can become outdated. Tell the user when a hint is likely stale, explain the evidence, and ignore or update it as appropriate. After merging coordinated PRs across repositories, review and update the affected dev hints to suitable refs for the merged work. Use `sources dev-hints [name] --ref branch|tag|commit` to capture from linked checkouts, or `sources dev-hints [name] --hint <git-ref>` for a literal ref. Omit `name` to update all sources. Review the `.sources.conf` diff and report updates. Hints are separate from release pins; never repin or switch an existing development checkout merely to follow a hint.

To capture a newly tested development combination, first commit and make source changes available upstream within the user's authorized scope. Dirty development edits are excluded from pins.

```sh
sources check
sources lock --check
git diff -- .sources.lock
```

Unqualified `sources lock` captures every development checkout's HEAD, regardless of active mode, without fetching. Adding `--check` ensures proposed commits and recursive submodule inputs are cached from configured upstreams, fetching missing inputs; cache presence does not prove current remote availability or test success.

For a partial update, replace the example commit with the intended full commit ID:

```sh
sources lock lib:<full-commit> --check
```

For a Git tag, use `sources lock lib:tag=v1.2.3 --check`; it stores the resolved commit, not a moving tag.

For sources with `pnpm` mappings, pin published versions with `sources lock --registry @example/ui:1.2.3 --check`. This downloads selected release archives into pnpm's store before writing; download failure preserves the lock. Without `--check`, registry pinning needs no network or pnpm. Git and package pins coexist in `.sources.lock` version 2, and either lock selection preserves the other. Git is the default; use `--git` explicitly for clarity.

`sources dev` selects local checkouts and `sources release --git` selects pinned checkouts through identical file overrides. Once file overrides and the pnpm lock agree, switching between them leaves dependency files untouched. Initial setup and returning from registry selection install with pnpm; refresh file packages explicitly when inputs change. `sources release --registry` selects locked published versions and always installs. It uses Git for sources without package mappings. Review and commit Sources pins, workspace overrides, and the pnpm lock together. `fetch` and `check-release` handle Git inputs for all sources.

Review the complete `.sources.lock`; commit approved pins with the app changes when authorized. For reproduction, retain the lock belonging to the selected app revision and do not run `lock` merely to activate it.

Use a clean app checkout at the intended revision for an online release build:

```sh
sources release --clean
sources status
git rev-parse HEAD
```

`--clean` discards all managed checkout contents, including edits, untracked files, dependency installations, and ignored outputs, then recreates pinned parent and recursive submodule checkouts from upstream. Preserve valuable managed edits elsewhere before using it. Original development checkouts are untouched.

For local switching, `sources release` retains nonconflicting ignored artifacts, obtains missing inputs, and refuses dirty managed sources or overwrite conflicts. Retained outputs may need rebuilding.

For offline reproduction, use `sources release --git --offline` or `sources release --registry --offline` only with complete cached parent and recursive submodule commits, trees, and file contents. It forwards `--offline` to pnpm when enabled, requiring its package inputs in the local store too. It cannot be combined with `--clean`. `sources fetch` or `sources check-release` can prepare caches online without activating or repinning.

For registry activation, release installs dependencies with lock updates allowed; verify the committed release state with `pnpm install --frozen-lockfile`. Otherwise install with the project's frozen package lock. Build and test with existing tooling. Sources does not clean the app's outputs; use a fresh app checkout or its established cleanup procedure separately. Record the app HEAD, source lock, tooling, and build/test results with release evidence.

For configured cross-repository Turbo builds, use `sources turbo run <task>` after source activation and dependency bootstrap. Parent preparation tasks invoke allowed exact pairs with `sources task <package>:<task>...`; use `sources-setup` for the allowlist and per-task hash wiring, and the [Turbo guide](https://github.com/refinio/sources#turbo-bridge) for details. Keep artifact/install refresh explicit and source selection, contents, and configuration stable during the build.

At the invoking project root, `sources turbo` snapshots every package/task in that project's Sources allowlist before launching parent Turbo. All those owners need active checkouts and installed local Turbo, even with a parent `--filter`; the filter does not narrow this root snapshot.

Explicit `sources task <package>:<task>...` selects owners from configuration and requires active checkouts and local Turbo for those selected owners. For either bridge command, each child's nested hash snapshot resolves only managed hashes consumed by its selected Turbo tasks, their transitive prerequisites, and global hashed environment declarations. Owners of those consumed hashes also need active checkouts and local Turbo. Unrelated allowlist entries do not require active checkouts merely for owner lookup. An unused reverse-edge checkout may remain unactivated when that selected closure and its scripts/install inputs do not require it; a repository cycle alone does not imply a task-hash cycle. All configuration must still validate.

For repository cycles, prepare required checkouts and selected dependency closures explicitly, from foundations toward consumers, using native filtered frozen installs where applicable. `sources dev` and `sources release` activate all direct sources of the invoked project, but do not recursively activate nested Sources projects; an unused direct checkout created by activation does not require preparing its own sources. Honor each immediate consumer's pins instead of substituting an ancestor checkout. See the [selected-closure bootstrap contract](https://github.com/refinio/sources/blob/main/docs/turbo-integration-spec.md#bootstrapping-selected-packages-in-repository-cycles).

A required owner selected as a pnpm registry release has no checkout: retain the intended registry selection and use its published-package build path. Do not switch selections or substitute an available development checkout to make a bridge invocation succeed. Neither bridge command selects modes, fetches inputs, installs dependencies, or updates pins.

Report failures and the suggested repair command; rerun activation after fixing the cause. Return to development links with `sources dev` when requested. Do not publish artifacts or create a release unless authorized. See the [README](https://github.com/refinio/sources#readme) for details.
