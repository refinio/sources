# Sources CLI specification

Status: behavior contract for the experimental first version.

## Purpose and scope

`sources` manages Git repositories consumed through stable local workspace paths. It lets developers use their existing working checkouts and lets release workflows use exact, committed source revisions.

The tool owns configuration, source commit pins, managed Git clones, and workspace symlinks. An opt-in pnpm integration also switches managed package references between local files and exact registry versions and runs pnpm installation. It does not build packages, run tests automatically, package applications, sign artifacts, or publish releases.

There are no Sources environment-variable overrides for source locations. pnpm retains its native registry, authentication, and installation configuration. Developers configure local paths in `.sources.local.conf`.

## Implementation and platform decisions

Implement the CLI in Rust and invoke the installed Git executable with separate arguments, without an implicit shell. Use Git's machine-readable output for inspection. Git remains an installation prerequisite; users do not need a Rust toolchain.

Linux and Apple silicon macOS are the supported platforms for version 1. Native Windows support is best effort, including directory junctions where appropriate. Platform limitations must produce useful errors. Public distribution publishes the advertised platform archives together after their builds and installation checks succeed.

Distribute prebuilt binaries with a single installation command for each platform. Shell and PowerShell installers can be generated with cargo-dist. Pin build dependencies and toolchains, and test the packaged executable as well as the development build.

### Public distribution and agent skills

Distribute Sources under the MIT license from `refinio/sources` on GitHub. Publishing a GitHub Release manually, including publishing a saved draft or prerelease, triggers native builds for Linux x86-64, macOS Apple silicon, and Windows x86-64. Check out the release tag and require its version to match the Cargo package version. Build and verify every archive before uploading binaries, checksums, and generated shell/PowerShell installers to that existing release. Ordinary pushes do not create releases. Read-only candidate builds may run on pull requests or manual dispatch without publishing assets.

The single-command installers use GitHub Releases to select a release and detect the computer's platform. The default command selects the latest stable release; versioned release URLs select an exact version or prerelease. Git remains required, but Rust and the GitHub CLI are not needed to install a binary. After successful installation, print `npx skills add refinio/sources`; do not run it automatically or print a success instruction after installation fails.

Shell installs with `SOURCES_NO_MODIFY_PATH=1`, or with `SOURCES_UNMANAGED_INSTALL` selecting an existing binary directory, preserve shell profiles and existing PATH environment scripts. Do not migrate or remove legacy `env`/`env.fish` scripts in these modes, including scripts referenced from custom profiles. Unmanaged installation implies no PATH modification without needing a second flag.

Ship two brief distributable agent skills. `sources-setup` applies when a user requests external local or symlink dependencies, or when an agent finds loose external local dependencies. Before adopting Sources in an unconfigured project, ask whether the user wants it; on acceptance, briefly explain and perform setup. Existing configured projects use the normal development workflow. `sources-release` covers capturing or updating fixed pins, selecting pinned sources for a release or local reproduction, and returning to development. These skills do not authorize publishing a consuming application's release.

Keep command parsing and output separate from configuration validation, Git operations, workspace link operations, and command execution. The command implementation owns the project lock and sequences these operations. No transaction journal, rollback system, or saved mode flag is required.

## Project discovery and command conventions

Search the current directory and its parents for the nearest `.sources.conf`, following Cargo's manifest-discovery convention. The containing directory is the project root. Do not search downward or combine configurations from multiple roots. Support a global `-C <directory>` option, like Git, to change the starting directory before discovery. Relative configuration paths always resolve against the discovered project root.

`sources setup` uses an existing discovered project when one exists. If no configuration is found, it creates the project in the starting directory. Other commands fail with a useful diagnostic when no project is found.

Support `--help`, `--version`, and `sources status --json`. JSON status has a versioned top-level schema and includes the same source state and diagnostics as human-readable status. Write command results to stdout and warnings and errors to stderr. Disable color and progress decoration for non-terminal output. Commands never require interactive input to proceed; errors suggest the exact command to retry or the configuration to edit.

Exit with 0 on success, 1 for operational or validation failure, and 2 for invalid CLI syntax. `sources run` preserves the child's exit status once it starts. On Unix, child termination by a signal uses the conventional `128 + signal number` exit status. With an accessible controlling terminal, inherit the shell's process group and let the shell manage terminal ownership, pipelines, suspension, and continuation. Hold the project lock until the direct child exits. Interactive commands must wait for their own workers; workers left behind after that child exits are outside the lock and supervision guarantee. Without a controlling terminal, create a dedicated child process group, forward interruption and termination to that group, and wait for its lifetime before releasing the lock. Jobs that leave that group are outside supervision. Windows uses native child exit behavior with best-effort interruption forwarding.

## Core guarantees

- Git source workspace references keep the same paths in development and release mode. Opt-in pnpm registry sources instead switch effective dependency references.
- Development mode links to user-managed checkouts. Release mode links to tool-managed checkouts at locked commits.
- Switching modes never changes `.sources.lock`; only an explicit `sources lock` operation changes Git or registry pins. Opt-in pnpm switching changes `pnpm-workspace.yaml` and may change `pnpm-lock.yaml`.
- The tool never resets, stashes, cleans, changes branches in, or deletes a development checkout.
- Managed checkouts are ordinary Git clones, with one persistent checkout per source. There is no separate bare repository.
- Normal release switching retains generated files and installed dependencies where Git checkout permits it.
- Everything inside `.sources/` is disposable managed state. `sources release --clean` recreates managed checkouts completely and may discard their source edits, untracked files, and generated artifacts.
- Link objects directly inside the configured workspace link directory are managed and replaceable. Their targets remain outside that ownership grant.
- Managed clones obtain source objects from their configured upstreams. Locally cached commits are sufficient evidence for pin availability; there is no independent proof of current remote availability.
- Managed activation includes recursive submodules at the commits recorded by their containing repositories.
- Network access follows the command policy below. Read-only local inspection does not fetch implicitly.

## Configuration files

All three files use UTF-8 JSON. The `.conf` suffix does not imply a shell or executable configuration format. Examples containing placeholder hashes or URLs must be replaced before use.

### `.sources.conf`

Committed repository configuration:

```json
{
  "version": 1,
  "directory": "./sources",
  "sources": {
    "one.experimental": {
      "url": "https://github.com/refinio/one-experimental.git"
    },
    "one": {
      "url": "<upstream repository URL>"
    }
  }
}
```

`directory` is the directory containing stable workspace links. It resolves relative to the directory containing `.sources.conf`. Each source name is both its configuration key and its directory name below that directory. For example, `one.experimental` maps to `sources/one.experimental`.

Source names must be valid single directory names. Reject path separators, `.` and `..`, and names that collide on the current filesystem. Source directories cannot overlap the internal `.sources` directory or overwrite configuration files.

`url` identifies the expected upstream repository and the remote used for managed clones. Configuration does not pin a moving branch as a release input; `.sources.lock` pins the exact commit.

### `.sources.local.conf`

Ignored local configuration:

```json
{
  "sources": {
    "one.experimental": "/home/arne/repos/one-experimental",
    "one": "/home/arne/repos/one"
  }
}
```

Paths may be absolute or relative to the configuration root. They refer to development checkout roots, including valid Git worktrees. They are literal paths, not shell expressions. The tool does not expand environment-variable expressions or execute configuration values.

Every declared source needs a local mapping for `sources check` and `sources dev`. An unqualified `sources lock` captures Git HEADs and needs mappings for all sources. Release operations do not require development mappings. Unknown source names in local configuration are configuration errors.

### `.sources.lock`

Committed release pins use lock format version 2:

```json
{
  "version": 2,
  "sources": {
    "one.experimental": {
      "git": "<full commit SHA>",
      "pnpm": {"@example/core": "1.2.3"}
    },
    "one": {"git": "<full commit SHA>"}
  }
}
```

Each source can retain both a Git commit and published package versions. Git values are full commit object IDs using the repository's object format. Registry values are exact semantic versions, including optional prerelease identifiers, never ranges or moving tags. Either selection may be absent during incremental pinning. Git release requires Git pins for every source. Registry release requires all package pins for sources with pnpm mappings and Git pins for remaining sources. Missing required pins and obsolete source/package entries fail validation.

This is an intentional format break. Version 1 string-valued source pins are not accepted. Convert each string to an object with a `git` field and set lock version 2, or remove the old lock and explicitly repin. Remove package `version` fields from `.sources.conf` and pin them with `sources lock --registry`.

The app repository's own commit is recorded by the release workflow. It is not placed inside its own `.sources.lock`, which would create a circular reference.

Lockfile writes use a temporary file and atomic replacement. A validation failure leaves the existing file unchanged. Ordering and formatting are deterministic so reviews show actual pin changes.

### Opt-in pnpm package releases

A source may declare a `pnpm` object mapping package names to paths within either its development or pinned Git checkout:

```json
{
  "url": "https://github.com/example/sdk.git",
  "pnpm": {
    "@example/core": {"path": "packages/core"},
    "@example/ui": {"path": "packages/ui"}
  }
}
```

Mappings make registry selection available without removing the Git checkout workflow. Package versions belong in `.sources.lock`. Paths must be relative, stay within the selected checkout, and contain a `package.json` declaring the mapped name. Names cannot be managed by multiple sources. pnpm file-reference paths require valid Unicode and reject literal Unix backslashes before link or configuration mutation. Git-only sources retain native-path support.

Require pnpm 10.15 or newer on PATH, a root `package.json`, and an ordinary root `pnpm-workspace.yaml` for activation. Keep mapped packages outside pnpm workspace membership. Consumers declare ordinary package dependencies. Sources owns root workspace `overrides` entries for mapped names across the dependency graph, including internal dependencies between managed packages. Conflicting scoped overrides and legacy `package.json` pnpm overrides require configuration changes before switching. Preserve unrelated workspace settings, overrides, and comments. pnpm retains registry URLs, scope mappings, authentication, and install settings.

`sources dev` selects development links; `sources release --git` selects pinned managed Git checkouts. Both use identical `file:` overrides through stable links. Once file references and their pnpm lock entries agree, switching between these checkouts does not run installation or rewrite workspace YAML or `pnpm-lock.yaml`. Initial setup, or returning from registry references, runs `pnpm install --no-frozen-lockfile`. Refresh dependencies explicitly with pnpm when manifests, files, or generated outputs change. If file installation fails after pnpm has written its lock, retry `pnpm install --no-frozen-lockfile` explicitly before reapplying the source selection. File packages follow pnpm installation semantics and do not guarantee live updates.

`sources release --registry` selects locked exact versions for mapped packages, removes their source links, and runs `pnpm install --no-frozen-lockfile`. Sources without package mappings still activate pinned Git checkouts. Registry-selected sources need no development mappings, Git pins, managed clones, or Git network access. Registry installation runs on every registry activation so failed installs can be retried. Commands hold the project lock through installation; failures may leave partial state and suggest rerunning the intended selection. No rollback is required.

Review and commit `.sources.lock`, release-mode workspace overrides, and `pnpm-lock.yaml` together. Sources never chooses `latest` or changes pins during activation. pnpm's lock records resolved dependencies and integrity; Sources' lock records the intended Git and package selections. CI can verify the selected package graph with `pnpm install --frozen-lockfile`.

`--offline` forwards pnpm's offline option for required installation and requires cached Git inputs and package inputs in pnpm's store. `--clean` removes disposable Sources state, including unused Git caches in registry mode, but leaves the app's store, `node_modules`, and outputs intact. pnpm lifecycle scripts follow project settings; offline installation does not sandbox them.

`status` JSON uses schema version 2 and combines actual links and overrides. `pnpm_selection` is `file`, `registry`, `mixed`, or `unconfigured`; `release_kind` is `pnpm` for an active registry selection and `git` otherwise. It reports both locked selections without asserting installed-package correctness. `run` and Turbo commands accept local or pinned Git checkouts. Registry selection requires `sources dev` or `sources release --git` first. `fetch` and `check-release` handle Git inputs for all sources, including those with package mappings.

## Filesystem layout and ownership

```text
project/
  .sources.conf
  .sources.local.conf
  .sources.lock
  .sources/
    checkouts/
      one.experimental/       # Ordinary managed clone, including .git
      one/                    # Ordinary managed clone, including .git
    project.lock              # Project-wide operation lock
    ...                       # Disposable temporary clones and caches
  sources/
    one.experimental -> ...
    one -> ...
```

In development mode, links point to the configured development checkouts. In release mode, they point to `.sources/checkouts/<name>`.

Generated workspace links, `.sources/`, and `.sources.local.conf` are ignored by the app repository. `.sources.conf` and `.sources.lock` are committed. Generated links must not make mode switching dirty the app repository.

Workspace configuration uses stable paths, for example:

```yaml
packages:
  - "packages/*"
  - "sources/one.experimental/packages/*"
  - "sources/one/packages/vger.ui"
  - "sources/one/packages/ui.theme"
```

Everything below `.sources/` belongs to the tool and may disappear. Users must keep development checkouts and valuable edits elsewhere. No ownership manifest or per-clone adoption procedure is required for contents in that directory. Existing valid clones still need the expected repository identity before reuse; clean recreation can remove invalid, incomplete, or mismatched contents there.

All link objects directly inside the configured workspace link directory belong to the tool, including stale links for sources removed from configuration. Activation may replace configured links and remove stale links. Unlink the link object itself without following it or deleting its target. Ordinary files and directories are not covered by this grant; refuse to overwrite them at required link locations and leave unrelated ordinary entries alone.

Reject path layouts that overlap configuration files, the internal directory, the workspace link directory, or development checkouts in ways that could cause managed deletion to affect development contents. Resolve existing path aliases when checking overlap. The internal directory and workspace link directory must be real directories, not links to other directories. Recursive deletion inside managed state must not follow links to external targets.

Use directory symlinks on Linux and macOS. Windows may use directory junctions for local directory targets. If links require platform-specific permissions or facilities, failures must identify the affected path and remedy. A failed link operation must not damage either checkout.

## Repository identity checks

A development checkout must exist, be a Git checkout rooted at the configured location, and have a remote matching the expected upstream. A fork checkout is acceptable if another remote, such as `upstream`, matches the configured URL.

Compare normalized repository identities. Recognize equivalent standard HTTPS and SSH URLs for the same host and repository, including the optional `.git` suffix. Do not require identical URL spelling, silently equate different repositories, or expose embedded credentials in output.

Managed clones use the configured upstream and are never seeded from development checkouts or their object stores. Reused managed checkout paths must be actual clone roots with Git metadata contained in managed state, not links or linked worktrees into development repositories. A mismatch between a managed clone's remote and current configuration is an error when reusing the clone. Suggest `sources release --clean` to recreate disposable state using the current configuration.

These checks establish configured repository identity, not release approval, branch ancestry, signature trust, or source quality. Those remain release-policy concerns.

## Cleanliness and generated artifacts

Every command inspects relevant existing checkouts before operating on them. Report tracked modifications, staged changes, and non-ignored untracked files. Include source names and changed paths in warnings.

Ignored files such as `node_modules`, `dist`, and other build outputs are allowed. Tracked files remain source changes even if a matching ignore rule exists. All files inside managed state are disposable under `--clean`, including tracked edits and non-ignored untracked files. Report this when clean recreation will discard changes.

Development checkouts may be dirty. Inspection, development linking, command execution, and locking warn but do not fail solely for this reason. A lock captures a commit, never the uncommitted working tree.

Managed reference checkouts are intended for fixed source revisions. Normal and offline release activation refuse dirty managed checkouts, including when the requested commit already equals HEAD. They do not stash or discard edits. `release --clean` explicitly discards disposable managed contents and does not require them to be clean or valid Git checkouts. Inspect initialized submodules recursively so their source edits cannot escape normal cleanliness checks.

`sources run` checks before and after the child command. If the child changes source files, report the resulting dirty state. Dirtiness alone does not replace the child's exit status.

Ordinary checkout may encounter ignored or untracked files that would be overwritten. It must fail safely rather than force the checkout or remove the files. Use `--no-overwrite-ignore` for parent checkouts and preserve the same behavior when switching submodule revisions.

## Network and fetch policy

| Command | Network behavior |
| --- | --- |
| `sources setup` | None. |
| `sources check` | None. |
| `sources dev` | No Git fetch; opt-in pnpm installation may use the registry. |
| `sources status` | None. |
| `sources run` | None by the tool; the child command may use the network. |
| `sources turbo` / `sources task` | No fetch, installation, or tool download; invoked Turbo tasks may use the network. |
| `sources lock` | None unless `--check` is present. |
| `sources lock --git --check` | Cache proposed commits and recursive inputs, fetching missing objects as needed. |
| `sources lock --registry --check` | Download proposed package releases into pnpm’s store before writing pins. |
| `sources fetch [name...]` | Fetch required revisions for all or selected sources, including recursive submodule inputs. |
| `sources check-release` | Ensure the complete locked inputs are cached, fetching missing objects as needed. |
| `sources release` | Obtain missing locked inputs, including recursive submodules, then activate. |
| `sources release --clean` | Recreate managed clones from their configured upstreams at the locked revisions. |
| `sources release --offline` | None; require complete locked inputs in existing managed caches, including submodules and file contents. |

All tool-initiated fetches operate on managed clones, their submodule repositories, or temporary managed clones. They never fetch into development checkouts. Fetching alone must not change the lockfile, active workspace links, or existing checked-out commits.

Fetch the required commits instead of downloading every branch, tag, and full repository history. Prefer shallow depth-one fetches of full commit object IDs, including all trees and file contents required for complete checkouts of those revisions. Use shallow history with complete checkouts, not partial clones, blob filters, or sparse checkouts. Apply the same shallow-fetch policy recursively to submodules. Do not fetch unrelated tags or force rewritten tags.

If the server cannot serve the required commit directly or shallowly, fetch advertised refs and deepen only as needed to obtain that commit. A broader history download is a compatibility fallback and must be reported. Fail with the source name and requested commit if the upstream cannot supply the missing input. Do not retry authentication or transport failures as though they meant unsupported shallow fetching.

A commit available in a valid managed clone with the configured upstream is sufficient for pin validation. There is no fresh-clone verification, ancestry proof, or requirement to prove that a cached commit is still downloadable upstream. A commit found only in a development checkout can be locked locally, but `lock --check` must ensure it is also available in the corresponding managed clone. Managed caches are trusted as tool-owned state rather than audited for manual Git object injection.

Commands that permit network access need not contact the upstream again for already cached inputs. Explicit tag selection with `sources lock <name>:tag=<tag> --check` contacts upstream to select its current tag target. `sources fetch` explicitly contacts each selected upstream even when the requested commit is cached. Cache readiness means the required inputs exist locally, not that a pin equals the latest branch head or remains advertised remotely.

Successful acquisition caches the complete trees and file contents for each selected revision. Local-only commands, including offline release, must fail on missing required objects instead of attempting a network request.

### Submodules and other Git extensions

Managed source acquisition always includes recursive submodules at the exact gitlink commits recorded by the selected parent revisions. Use checkout behavior, not merges or submodule branch tracking. Resolve submodule URLs using Git's normal `.gitmodules` and relative-URL rules, and synchronize changed URLs in managed clones. Do not use `git pull` or `submodule update --remote`.

Fetch and readiness commands cache required recursive submodule inputs without moving existing checkout HEADs. Activation initializes and checks out the required submodules before reporting success. Missing submodules, missing gitlink commits, and submodule fetch failures identify both the source and submodule path. A mismatched submodule HEAD is incomplete activation, not automatically a source edit; refuse actual tracked or non-ignored untracked edits during normal activation. Changes to submodule membership that cannot be applied without losing existing files fail and suggest `sources release --clean`.

Development commands inspect existing submodules but do not initialize, update, synchronize, or fetch them. Offline release requires all recursive submodule inputs and working-tree file contents to be cached and uses no-fetch checkout behavior.

Version 1 adds no special orchestration, validation, or guarantees for Git LFS, custom hooks, or filters. They do not expand the source lock format or the tool's responsibilities.

## Command specifications

### `sources setup`

Create missing configuration files, internal directories, and ignore entries. Initial source maps may be empty. Preserve existing configuration and never overwrite user entries.

No repositories are cloned or fetched. No mode is activated. Report which files were created and which require editing.

### `sources check`

Validate configuration and all configured development mappings. Check existence, Git checkout roots, and expected upstream identity. Report dirty development sources as warnings. Return failure for invalid configuration, missing checkouts, or repository mismatches.

This command does not require a complete release lock and does not validate remote commit availability. It does not repair links or fetch.

### `sources check-release`

Require a complete valid lock. Validate and inspect existing managed clones, initialize missing clones, and ensure every locked commit and its recursive submodule inputs are cached. Fetch missing inputs using the minimal-fetch policy.

Report dirty managed clones as errors for release readiness. A dirty development checkout can be reported as a warning when inspected but does not invalidate an independent pinned release checkout.

Do not move existing managed worktree HEADs, including submodule HEADs, or change workspace links. Cache creation and Git object/ref updates are permitted. A new managed clone may remain without an activated worktree until release is run. Success means the locked inputs are cached and existing materialized managed checkouts contain no source changes. It does not establish current remote availability or assert that links already select release mode.

### `sources dev`

Perform development validation for every source before changing any link. Point stable workspace paths to configured development checkouts. Warn about dirty development sources and any relevant existing managed reference checkout.

Do not change source branches, fetch, update pins, or modify checkout contents. Repeating the command when links are already correct is harmless.

### `sources release [--git | --registry]`

Git is the default. `--git` and `--registry` are mutually exclusive. Require complete pins for the requested selection. Registry activation follows the pnpm rules above; the checkout rules below apply to Git-selected sources. Check managed checkout identity and cleanliness, and ensure all locked inputs are cached before starting activation. Fetch missing inputs using the minimal-fetch policy.

Check out each locked commit in detached mode in `.sources/checkouts/<name>` and initialize and update recursive submodules to their recorded commits. Then point workspace links to those checkouts and remove stale link objects. Create missing managed clones as needed. A failure can leave partial activation; report it and suggest the appropriate command to rerun.

Retain generated artifacts and installed dependencies. Do not run `git clean`, `reset --hard`, automatic stashing, or builds. pnpm installation only follows the selection rules above. Never force past a checkout conflict. Warn that retained outputs may require rebuilding.

### `sources release --clean`

Recreate the entire set of managed checkouts to force rebuilding dependencies and generated outputs. It is the required mode for the release environment and the repair command for broken managed clones. It may discard every file in those checkouts, including source edits. No interactive confirmation is required for this explicit operation on disposable state.

1. Validate configuration, complete pins, and path safety for the whole operation before deleting managed contents.
2. Report source edits in inspectable managed checkouts as changes that will be discarded. Broken clones and identity mismatches do not block clean recreation.
3. Remove and recreate managed checkouts from the configured upstreams using minimal fetches. Old managed contents need not be retained while downloading replacements.
4. Check out locked parent commits in detached mode and initialize and update all recursive submodules at their recorded commits.
5. After all checkouts are ready, activate release links, remove stale link objects, and clear obsolete managed temporary state. Preserve the live project lock file while holding the lock.

A network, checkout, or link failure may leave missing checkouts, broken links, or a partially activated source set. Do not restore old clones, create rollback backups, or maintain a transaction journal. Report what failed and suggest rerunning `sources release --clean`, or `sources dev` to restore development links.

Successful recreation removes all prior files in the managed clones, including tracked edits, untracked files, ignored artifacts, dependency installations, and old local Git cache state. It never deletes or cleans development checkouts or the app repository's own outputs.

### `sources release --offline`

Use existing managed clones and complete locked inputs without network access. Fail before activation if any required parent or recursive submodule commit, tree, or working-tree blob is missing. Disable submodule fetching. Apply the same cleanliness, checkout, and link rules as normal release mode.

Report explicitly that cached inputs were used without network access. This mode supports local reproduction; release environments use clean recreation from upstream. `--offline` and `--clean` are mutually exclusive.

### `sources fetch [name...]`

Fetch all configured sources when no names are given, or only the named sources. Create missing managed clones. Check identity and warn about source changes in existing clones. For each source, fetch its locked commit if a pin exists, otherwise the upstream's default-branch tip. Fetch required recursive submodule inputs too. Contact every selected upstream even when the requested commit is already cached, but do not require independent proof that a cached object was transferred again.

Update Git objects and fetched refs without changing existing parent or submodule worktree HEADs, workspace links, or pins. Newly initialized caches need not have activated worktrees. Unknown names are errors. A failed fetch returns failure and identifies the affected sources.

### `sources lock [--git] [<name>:<hash> | <name>:tag=<tag> ...] [--check]`

Git is the default. With no positional arguments, capture HEAD from every configured development checkout, regardless of active mode or pnpm mappings. Preserve existing registry pins. Perform development identity checks and warn prominently for dirty checkouts. The warning must state that uncommitted changes are excluded.

With positional arguments, update only the named pins and preserve others. Accept full commit hashes or unambiguous abbreviated hashes and always store full commit object IDs. Resolve commits using configured development checkouts or managed clones. Ambiguous identifiers and objects that are not commits fail.

Use the explicit `<name>:tag=<tag>` selector to select an exact lightweight or annotated Git tag, including the tag associated with a GitHub Release. Peel the tag to a commit and store only its full commit object ID; release assets and moving tag names are not stored in the lock. Branch names and arbitrary revision expressions are not selectors.

Without `--check`, resolve the exact tag using validated development checkouts and managed tag caches without fetching. Differing local targets are ambiguous and require `--check` or an explicit commit ID. With `--check`, contact the configured upstream and shallow-fetch only that exact tag into managed state, even when a local tag exists. Upstream selection is authoritative; preserve development repositories and ordinary local tag refs. Cache the selected commit and complete recursive inputs without moving parent or submodule HEADs. Later activation uses the immutable pinned commit, even if the tag moves.

Without `--check`, do not fetch. Fail if a supplied hash cannot be resolved locally. Do not require local development mappings for explicitly pinned sources if their hashes can be resolved from managed clones.

With `--check`, ensure proposed commits and their recursive submodule inputs are available in the corresponding managed clones before writing. Managed cache presence is sufficient; fetch missing inputs from the configured upstreams. Full hashes may identify commits not yet available locally. Abbreviations must resolve unambiguously among locally available objects; do not download unrelated history just to search for an abbreviation. Ask callers to supply a full hash when needed. For a partial update, validate the changed pins; `sources check-release` remains the complete-lock validation command.

Validate the entire requested update before a single atomic lockfile write. Duplicate source arguments, unknown source names, failed resolutions, or required fetch failures leave the previous lock unchanged. Locking alone does not fetch unless requested, move existing checkouts, or change mode.

### `sources lock --registry <package>:<version>... [--check]`

Require explicit managed package names and exact semantic versions. Update only the named package pins, preserving all Git pins and other package pins. `--git` and `--registry` are mutually exclusive. Reject unknown packages, duplicates, ranges, and tags before writing. Without `--check`, validate locally without contacting pnpm or the registry.

With `--check`, invoke `pnpm store add <package>@<version>...` from the app root to download the selected releases using pnpm's registry/authentication settings before writing. Download failures leave `.sources.lock` unchanged. Cache population is allowed; do not switch links or overrides, install app dependencies, or change `package.json` or `pnpm-lock.yaml`. Cached packages can satisfy checks without an independent proof of current remote availability. This downloads selected archives; pnpm installation resolves the full dependency graph later.

Validate the entire update before one atomic write. Neither lock command activates a selection.

### `sources status [--json]`

Inspect local state without fetching. Show each source's configured upstream, stable workspace path, actual link target, development path if configured, active commit if available, locked commit, dirty state, and whether active and locked commits match.

Derive mode from actual link targets instead of trusting a saved mode flag:

- `dev`: every link targets its configured development checkout.
- `release`: every link targets its managed checkout. Report commit mismatches separately; links alone do not prove a valid release.
- `mixed`: configured links select different modes or include unexpected targets.
- `unconfigured`: no sources are declared or no source links have been activated.

Report missing and broken links, invalid managed checkouts, missing commits, unavailable checkouts, and missing or mismatched recursive submodules explicitly. Include a suggested repair command. Report local cache readiness without claiming fresh remote verification. Errors in activated state or errors that prevent reliable inspection return failure; ordinary dirtiness is a warning. Valid caches without activated worktrees are not corrupt merely because no release checkout has been selected yet. Report mixed mode and active/locked commit mismatches without treating a valid development commit that differs from the lock as an inspection failure.

### `sources run <name> -- <command> [args...]`

Resolve the named source's active workspace link and run the child command there. Refuse a missing link, unexpected target, invalid repository identity, or unknown source name.

Execute the command and arguments directly, without implicit shell parsing. Users who need shell expressions can explicitly run a shell. Inherit environment, standard input, standard output, and standard error. On Unix, an accessible controlling terminal selects interactive execution even when standard streams are redirected or the job starts in the background. Keep the shell's process group and never change terminal foreground ownership. Terminal signals reach the shell job group naturally. Sources absorbs interactive `SIGINT` while waiting, so programmatic interruption must target the job group rather than the Sources PID. Forward `SIGTERM` explicitly sent to Sources to its direct child. Wait for the direct child and require interactive commands to await their own workers. Without a controlling terminal, supervise a dedicated child group and wait for the group, including remaining members after the direct child exits.

Inspect source cleanliness before and after execution. Return the child's exit status when it ran, including when it left source changes. Failure to start the child is a tool error. No automatic mode switch, fetch, dependency installation, or lock update occurs.

### `sources turbo [Turbo arguments...]`

Snapshot all configured child package/task hashes through each active checkout's local Turbo dry-run JSON. Require executable tasks and use their exact hashes, never a Sources-generated tree hash. Export each as `SOURCES_<NORMALIZED_PACKAGE>__<NORMALIZED_TASK>` to the parent's locally installed Turbo, forwarding arguments from the project root. Components uppercase ASCII alphanumeric characters, replace maximal punctuation runs with `_`, and trim `_`; reject empty components and collisions globally. `packages` is optional and additive to configuration version 1.

`SOURCES_` is reserved. Remove inherited keys before child queries, child execution, and parent launch. Inject the complete fresh snapshot into the parent. When a child has its own Sources configuration, resolve only its selected/transitive task graph's consumed managed hashes and use that child snapshot consistently for its final metadata query and execution. Follow checkout/task identities and reject genuine hash cycles; independent tasks may cross a repository cycle. Prefix matching follows case-insensitive environment naming on Windows. Preserve unrelated environment. Child queries and actual nested executions therefore use the same bridge environment even if child Turbo configuration includes a `SOURCES_*` wildcard. Single-package repositories use bare task names after validating the root manifest name; monorepo root tasks use Turbo's `//#task` identifier.

Hold the project lock during snapshot preparation, then release it before launching parent Turbo. Parent tasks may invoke concurrent `sources task` prerequisites, which serialize through the same project lock. Keep source selection, configuration, and checkout contents stable for a parent build; the snapshot does not freeze mutable files. Preserve the existing interactive/noninteractive supervision policy without reacquiring the outer lock. No implicit fetch, build-output staging, dependency installation, or publishing occurs. Turbo remains responsible for graphs, inputs, outputs, execution, and caching. See [the Turbo bridge contract](turbo-integration-spec.md) for parent task/env examples and explicit install-refresh prerequisites.

### `sources task <package:task>...`

Run explicitly allowed child Turbo entry points from active source checkouts. The optional per-source `packages` object maps actual Turbo package names to objects with distinct nonempty `tasks` lists. Package ownership and normalized environment identities must be unique. Existing `pnpm` mappings remain independent. Reject unknown fields, package names containing `__`, and names that cannot be represented by the selector/task identity.

Parse selectors at the first colon, preserving colons within task names. Validate every selector, active checkout, and selected executable Turbo task before running any task. Invoke each repository's locally installed Turbo via `pnpm exec turbo`; missing tools or bootstrap are prerequisites, never permission to download a fallback. Use exact package/task pairs and let Turbo schedule its prerequisites. Across sources execute sequentially in source-name order, stopping at the first failure. Preserve the Turbo process's output, status, and existing process supervision. Warn about source changes before and after execution. Do not fetch, switch modes, install dependencies, or update pins.

A registry release has no active checkout and requires `sources dev` or `sources release --git` first. See [the Turbo bridge contract](turbo-integration-spec.md) for configuration, hashing, and parent workflow details.

## Failure handling and concurrent operations

Validate configuration, path safety, and relevant inputs before changing links or replacing checkouts. Multi-source changes are not atomic. No rollback, transaction journal, saved previous mode, or backup-clone system is required. A failed command may leave partial state and must never claim success.

Inspect the state relevant to each command on the next invocation. Detect obvious breakage such as invalid Git roots, incomplete checkouts, broken links, or missing required submodule inputs. Error with the affected source and path and a concrete repair suggestion. `sources dev` repairs development links, `sources release` reapplies pins to reusable clones, `sources fetch` obtains missing inputs, and `sources release --clean` recreates broken managed contents. Commands performing the suggested repair may fix the corresponding missing or incomplete state instead of rejecting it first. Do not perform a full repository integrity audit on every invocation.

Use one exclusive operating-system advisory lock per project at `.sources/project.lock`. `sources turbo` holds it only through its child hash snapshot and releases it before parent execution. All other project commands acquire it before inspection or mutation, including `sources run` until its direct child exits in an interactive terminal context, or until its dedicated child group finishes without a controlling terminal. `--help` and `--version` need no project lock. Commands fail immediately when the lock is busy and suggest retrying after the other command finishes, except `sources task`, which waits for the same exclusive lock to serialize child Turbo execution. Do not support nested project operations inside `sources run` or inside a child `sources task` execution.

Use a lock held by an open file handle so process termination releases ownership. Do not treat the continued existence of the lock file as a live lock, and do not unlink or replace that file while an operation holds it. Creating the internal directory and lock file is permitted when bootstrapping a project. External editors and manual deletion of managed state are outside the lock's protection; users must not remove the lock file or managed directory during a running command.

Use nonzero exit status for configuration errors, missing prerequisites, repository mismatches when reusing clones, required fetch failures, dirty managed inputs during normal release, unsafe path collisions, and incomplete operations. Warnings alone do not fail a command unless its command-specific release rules require it.

## Common operation sequences

### First development setup

```sh
sources setup
# Edit .sources.conf and .sources.local.conf.
sources check
sources dev
sources status
pnpm install
```

Commit repository configuration. Keep local mappings and caches ignored. Configure workspace references once to use the stable paths.

### Daily development

```sh
sources dev
sources run one.experimental -- pnpm build
pnpm build
```

Edit development checkouts normally. Dirty-source warnings can be expected while working. Reinstall dependencies when package manifests or the selected package graph change. Pins do not change automatically.

### Capture a tested source combination

Commit and push changes in all contributing repositories first, then:

```sh
sources check
sources lock --check
git diff -- .sources.lock
```

Review and commit the lock with the app changes. Resolve dirty-checkout warnings before considering the combination a release candidate. Managed cache availability does not establish current remote availability, branch approval, or test success. Clean release recreation obtains inputs from upstream again.

### Update one source pin

```sh
sources lock one.experimental:<commit> --check
sources release
sources status
# Install dependencies, build, and test.
git diff -- .sources.lock
```

Commit the pin change after validation. Resume development with `sources dev`.

### Reproduce a pinned app revision locally

```sh
git switch <branch-or-tag>
sources release
sources status
# Install dependencies and rebuild as needed.
```

The selected app revision determines the configuration and lock. Generated outputs survive normal source switching, so this sequence does not establish a clean-build result. Use `--offline` only when all inputs are already cached and network access is unnecessary.

### Prepare an online release build

Start with a clean app checkout at the exact intended release commit or tag:

```sh
sources release --clean
sources status
# Install dependencies using the frozen package dependency lock.
# Build, test, package, and sign.
```

There is no need to run `sources check-release` immediately afterward: clean release already obtained the pinned inputs from upstream and activated their recursive submodules. Use `check-release` separately for local readiness checks without activation.

The release workflow must also clean the app's own outputs or use a fresh app checkout. Record the app commit and source lock with release evidence, and publish the artifacts that passed validation. Dependency archives, build tools, configuration inputs, and signed artifacts require separate retention for durable replay.

### Inspect or repair workspace wiring

```sh
sources status
sources check
sources dev
# Alternatively, use sources release to restore pinned reference links.
```

Repairing links never changes pins. To keep edits made in a managed reference checkout, copy or commit them into a development checkout before running `sources release --clean`. Managed contents are disposable and clean recreation will discard them. After an interrupted activation, rerun `sources release` if clones remain usable, or `sources release --clean` if they are broken. Use `sources dev` to restore development wiring independently of release-cache repair.

## Reproducible test environment

Use Rust integration tests with `tempfile` for isolated directories and `assert_cmd` for invoking the actual CLI executable. Build fixture repositories with the installed Git executable. Use real filesystem links, Git worktrees, and subprocesses; mock neither Git nor the filesystem in command acceptance tests. Pure configuration, URL normalization, and path-validation tests can run without subprocesses.

Each scenario creates an app project, at least two source upstreams, development clones, and managed state. Include a fork checkout with a matching upstream remote, nested submodules, and revisions that change submodule pins and URLs. Serve fixture upstreams on loopback using Git's upload-pack protocol, initially through `git daemon`. Cover HTTP and SSH with focused transport tests rather than requiring a hosted forge or public GitHub access. Exercise shallow fetches and compatibility fallback when a required commit cannot be obtained directly at depth one.

Isolate user and system Git configuration, identity, credential prompts, hooks/templates, and inherited repository-selection variables. Fix author and committer identities and timestamps, default branch names, line-ending settings, locale, and fixture file bytes. Pin the main CI toolchain and Git version and include a supported-minimum Git job. Preserve the fixture and a redacted command log when a test fails.

Run the required suite natively on Linux and Apple silicon macOS. A Linux container can provide an additional repeatable baseline but cannot substitute for native filesystem tests. Run a best-effort Windows job for directory links, path handling, and child execution. Track Windows failures without blocking supported-platform releases.

Exercise interruption, network failure, and concurrent operations using explicit synchronization points rather than timing sleeps. After partial activation, assert that the next relevant command reports breakage and a repair command, then assert that rerunning that command repairs it. Do not assert rollback or preservation of disposable managed edits during clean recreation.

Inspect fetched refs and object graphs to verify that shallow acquisition avoids unrelated histories when the server supports it and does not configure partial clones or sparse checkouts. Verify complete file contents for the selected parent and recursive submodule revisions. Tripwire the local server for local-only commands, and use network-denied execution where available to catch accidental fetching in offline mode. Include a cached commit removed from upstream: cached validation may succeed, while fresh clean recreation must fail if upstream can no longer supply it.

## Acceptance criteria

- Setup is repeatable and preserves user configuration.
- Git dev/release switching preserves workspace package paths, file references, and the existing pnpm lock. Git and registry pins coexist in lock version 2; lock flags update each independently. Registry `--check` downloads selected archives before atomic pin writes. Explicit registry activation installs exact package versions and may update the pnpm lock; returning to file references installs too.
- Mode reporting reflects real links, including manual changes and broken targets.
- Development checkouts remain untouched by fetches, checkout switching, and clean release operations.
- Locking supports complete local capture, partial explicit updates, full-hash normalization, dirty warnings, and validation before atomic writes.
- A commit in the expected managed cache passes pin validation without independent remote-availability proof; a commit found only in a development checkout does not satisfy `--check` until it is cached from upstream.
- Offline release works with complete parent and recursive submodule caches and makes no network requests.
- Normal release switching retains non-conflicting ignored artifacts and refuses source loss.
- Clean release recreates disposable managed state, including tracked edits, untracked files, ignored artifacts, and broken clones, without affecting development checkouts.
- Partial failures return failure, report affected paths, and suggest a command that repairs the detected breakage on rerun; rollback is not required.
- Managed acquisition prefers exact shallow revisions with complete file contents, including recursive submodules, with reported compatibility fallback when required. It does not use partial clones, blob filters, or sparse checkouts.
- Submodules activate at recorded gitlink commits, including nested modules and changed URLs; no branch merges or updates to moving submodule tips occur.
- Workspace link replacement and stale-link removal never remove link targets; ordinary files and directories at required link paths cause errors.
- `sources run` executes in the active source, preserves child exit behavior, and reports source changes after execution.
- Checkout operations and `sources run`/`sources task` execution exclude concurrent checkout mutation. Parent Turbo execution requires stable checkout selection and configuration after its hash snapshot releases the lock.
- One project lock fails immediately when busy except for serialized `sources task` calls, which wait. It becomes available after process termination without deleting a stale lock file.
- Discovery uses the nearest parent configuration, `-C` changes the search start, and JSON status and exit codes follow the documented command conventions.
- Linux and macOS tests are required; Windows support and tests are best effort.

## Design references

- [Cargo manifest discovery](https://doc.rust-lang.org/cargo/commands/cargo-locate-project.html)
- [Git command conventions and `-C`](https://git-scm.com/docs/git)
- [Git shallow clones](https://git-scm.com/docs/git-clone)
- [Git recursive submodule checkout](https://git-scm.com/docs/git-submodule)
- [cargo-dist installer options](https://axodotdev.github.io/cargo-dist/book/reference/config.html)

## Implementation boundary

Implementation of version 1 in the standalone refinio/sources repository was authorized on 2026-10-20. Public repository visibility, MIT licensing, installers, agent skills, and native binary publication when a release is manually published were subsequently authorized. On 2026-10-21 the user authorized named Git release selection and pnpm integration that switches local file dependencies to published releases, including automatic pnpm installation and dependency lockfile changes. The user subsequently authorized independent Git and registry pins in the committed lock, explicit `--git`/`--registry` flags on lock and release, and downloading registry releases with `lock --registry --check`. This setup does not create a release or migrate consuming workspaces by itself. Integrating the tool into consuming workspaces requires replacing their external workspace paths and tracked symlink assumptions with stable managed paths, and making their package dependency locks reproducible in that layout.
