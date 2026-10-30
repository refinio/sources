# Sources

`sources` selects development checkouts or exact release commits through stable workspace paths. Git source workspaces keep the same paths when switching between them. Optional pnpm integration switches local file dependencies to published package versions. Sources invokes the installed Git executable. Optional Turbo integration delegates builds to each repository's native Turbo; Sources does not publish releases.

Linux and Apple silicon macOS are supported. Native Windows is best effort. Use Git 2.39.5 or newer, with the SSH and credential setup your repositories require. The CLI is experimental and currently promises no backwards compatibility.

## Install

Install the latest stable GitHub Release on Linux x86-64 or Apple silicon macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/refinio/sources/releases/latest/download/sources-installer.sh | sh
```

On Windows x86-64, run in PowerShell:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -Command "irm https://github.com/refinio/sources/releases/latest/download/sources-installer.ps1 | iex"
```

The installers select the archive for your computer and install to Cargo's binary directory without requiring Rust or the GitHub CLI. SHA-256 checksums are published with the archives. The shell installer verifies them when `sha256sum` is available; PowerShell does not verify them automatically. Git must be on `PATH`. Follow the printed `PATH` instructions if needed. Use a release's versioned installer URL instead of `latest/download` to select an exact version or prerelease.

These download commands require a published release whose build workflow has finished. Before the first release, build from a checkout with Rust:

```sh
cargo install --path . --locked
sources --version
```

The repository pins Rust in `rust-toolchain.toml`. Rustup selects it automatically.

To update an installed CLI on Linux or macOS:

```sh
sources update
```

This installs the latest stable GitHub Release into the running binary's directory using the release's shell installer. It requires `curl`, `sh`, and write access to that directory, and works outside a configured project. After success it prints `npx skills update` to refresh installed agent skills separately. On Windows, close Sources and rerun the PowerShell installation command above.

`SOURCES_UPDATE_URL` can select an alternative trusted shell installer URL, such as a mirror. This executes the downloaded script. The default is the official latest stable release; the installer's existing download overrides still apply to its archives.

## Agent skills

After a successful binary installation, the installer prints this optional next step:

```sh
npx skills add refinio/sources
```

Choose the skills and agents to install to. This command uses Node/npm and does not run automatically during CLI installation.

For global agent access, use `npx skills add refinio/sources --global`. If an agent cannot install skills, it can read the same bundled instructions directly from the CLI:

```sh
sources skills
sources skills sources-setup
sources skills sources-release
```

The overview lists each skill's purpose. A named command prints the complete `SKILL.md` shipped with that CLI version, including its metadata. These commands work offline, need no project configuration, and do not install or change files. `sources --help` points agents to both options. Run `npx skills update` separately to refresh installed skills.

- [sources-setup](skills/sources-setup/SKILL.md) applies to external local dependency or symlink requests and existing loose checkout references. It asks before adopting Sources, then guides setup and daily development.
- [sources-release](skills/sources-release/SKILL.md) covers capturing or updating fixed pins, release/reproduction workflows, and returning to development.

## Configure a project

Run this from the consuming project's root:

```sh
sources setup
```

Setup creates missing configuration and ignore entries without downloading repositories. Existing configuration stays intact. Edit `.sources.conf` to declare upstreams and the workspace link directory. All configuration files contain UTF-8 JSON.

```json
{
  "version": 1,
  "directory": "./sources",
  "sources": {
    "library": {
      "url": "https://github.com/your-org/library.git",
      "dev_hint": "main"
    }
  }
}
```

Replace the example URL. Each source name is one directory name. For example, `library` appears at `sources/library`. Point your Cargo, pnpm, or other workspace configuration at these stable paths.

Optional `dev_hint` suggests a Git branch, tag, or commit for development setup. Hints can become outdated; ignore or update them when they no longer fit the work, and tell the user when a hint appears stale. Review and update affected hints after merging coordinated PRs across repositories. Hints do not change checkouts or release pins.

Capture hints from the currently linked checkouts, or supply a literal Git ref:

```sh
sources dev-hints --ref branch                  # Update all sources
sources dev-hints library --ref tag             # Require one exact tag at HEAD
sources dev-hints library --ref commit          # Record the full commit ID
sources dev-hints library --hint next/develop   # Set a hint without a checkout
```

Omit the source name to update all configured sources. Choose exactly one of `--ref branch|tag|commit` and `--hint <git-ref>`. Capture validates all selected links before one atomic configuration write. A detached HEAD cannot supply a branch hint; missing or multiple tags require a manual hint or another ref kind. Manual hints are stored without resolving them. Local mappings and `.sources.lock` remain unchanged.

Edit the ignored `.sources.local.conf` to map source names to development checkouts:

```json
{
  "sources": {
    "library": "../library"
  }
}
```

Paths resolve against the project root and may be absolute. Paths are literal: Sources does not expand environment variables or shell expressions. A development checkout must have a remote matching its configured upstream. Forks work when an `upstream` remote matches. HTTPS and SSH spellings of the same repository are equivalent.

```sh
sources check
sources dev
sources status
sources run library -- cargo test
```

Sources discovers the nearest `.sources.conf` in the current directory or a parent. Use `sources -C /path/to/project status` to start discovery elsewhere.

Commit `.sources.conf` and `.sources.lock`. Setup adds `.sources/`, `.sources.local.conf`, and the generated workspace link directory to `.gitignore`.

## Pin and select release inputs

Commit and push your source changes, then capture the development commits:

```sh
sources lock --check
git diff -- .sources.lock
```

The generated lock is JSON with `version: 2` and a `sources` object mapping source names to objects containing `git` commit IDs and optional `pnpm` package versions. Review and commit it with the consuming app changes. A lock captures committed revisions only. Dirty development checkouts produce warnings; their uncommitted edits are excluded.

To update one pin, replace `<commit>` with a full or locally unambiguous abbreviated commit ID:

```sh
sources lock library:<commit> --check
sources release
sources status
```

To select a lightweight or annotated Git tag, including a GitHub Release's tag:

```sh
sources lock library:tag=v1.2.3 --check
```

`--check` selects the configured upstream's exact tag and caches its inputs without changing existing checkout HEADs. The lock stores the peeled full commit ID, so later tag movement does not change release inputs. Without `--check`, tag selection uses existing local tags and managed tag caches without fetching; conflicting local targets fail. Branch names, arbitrary Git revision expressions, and GitHub release assets are not tag selectors.

Only `lock` changes pins. Neither mode switching nor fetching rewrites the lock. Normal release switching retains non-conflicting ignored artifacts, so rebuild dependencies and outputs when needed.

For a release build, start with a clean consuming app checkout and run:

```sh
sources release --clean
sources status
# Install dependencies, build, test, package, and sign using your normal tools.
```

`--clean` recreates managed source checkouts and discards all their contents, including source edits, untracked files, dependencies, and ignored outputs. It does not clean the consuming app's own outputs. Record the app commit and `.sources.lock` with release evidence.

## pnpm local and published packages

For a pnpm project, declare package names and paths inside the source repository:

```json
{
  "url": "https://github.com/example/sdk.git",
  "pnpm": {
    "@example/core": {"path": "packages/core"},
    "@example/ui": {"path": "packages/ui"}
  }
}
```

Place this entry in `.sources.conf`'s `sources` map. Git pins and published versions live together in the committed `.sources.lock`:

```json
{
  "version": 2,
  "sources": {
    "sdk": {
      "git": "<full commit SHA>",
      "pnpm": {"@example/core": "1.2.3", "@example/ui": "2.0.0"}
    }
  }
}
```

Update either selection independently:

```sh
sources lock --git sdk:<commit> --check
sources lock --registry @example/core:1.2.3 @example/ui:2.0.0 --check
```

Git is the default for `lock` and `release`. Git locking also accepts `sdk:tag=v1.2.3`; without arguments it captures all development HEADs, preserving package pins. Registry locking requires explicit package versions and preserves Git pins. Registry `--check` downloads the selected archives into pnpm's store before writing; failures leave `.sources.lock` unchanged. It does not install app dependencies or change the pnpm lock. Without `--check`, registry locking is local and does not require pnpm. Registry URLs, scope mappings, and authentication remain in pnpm configuration.

```sh
sources dev                 # Local checkout through stable file: paths
sources release --git       # Pinned checkout through the same file: paths
sources release --registry  # Locked published versions, then pnpm install
```

Require pnpm 10.15+, root `package.json`, and `pnpm-workspace.yaml`. Keep mapped packages outside workspace globs. Consumers retain ordinary package dependencies; Sources manages their YAML `overrides` across the dependency graph. Move legacy `package.json` pnpm overrides into workspace YAML first. Unrelated settings and comments are preserved. pnpm file paths must be valid Unicode and cannot contain literal Unix backslashes.

Initial file setup and transitions between registry and file references run `pnpm install --no-frozen-lockfile` and may update `pnpm-lock.yaml`. Once file overrides and the pnpm lock agree, local-to-Git and Git-to-local switching changes only the checkout links and leaves both dependency files untouched. Refresh installed file packages explicitly after manifest or generated-file changes. File references do not guarantee live updates.

Registry activation always installs, allowing failed installs to be retried. Sources without package mappings retain pinned Git checkouts in registry mode. Switching never changes `.sources.lock`. Review and commit the selected workspace configuration and pnpm lock alongside Sources pins; CI can verify with `pnpm install --frozen-lockfile`.

Registry activation needs no local mapping or Git pin for mapped sources. `--offline` uses pnpm's store and Git caches. `--clean` discards Sources state but leaves app dependencies, store, and outputs to existing tooling. `fetch` and `check-release` handle Git inputs for all sources. `status` reports both pins and selected references, not installed-package correctness. Checkout commands work with local or pinned Git selections; use `sources dev` or `sources release --git` before running them for registry-selected sources.

This intentionally replaces lock format 1. Convert old source strings to objects with a `git` field and set lock version 2, or remove the old lock and repin. Remove package `version` fields from `.sources.conf` and pin them with `sources lock --registry`.

## Turbo bridge

Optionally declare allowed child package tasks in each source's `.sources.conf` entry:

```json
"packages": {
  "@example/protocol": {"tasks": ["build", "pack:app"]},
  "@example/ui": {"tasks": ["pack"]}
}
```

After selecting checkouts, run exact task pairs:

```sh
sources task @example/protocol:build @example/ui:pack
sources task @example/protocol:pack:app
```

Install pnpm and bootstrap Turbo locally in each child repository first. Sources invokes `pnpm exec turbo`, validates every selected task before execution, and lets Turbo own the graph and cache. Task commands wait for the project lock so concurrent parent preparation tasks serialize. Registry-selected sources require `sources dev` before checkout tasks. Launch parent Turbo through Sources to receive all configured child task hashes:

```sh
sources turbo run build --filter=@app/web
```

For example, `@example/protocol:pack:app` exports `SOURCES_EXAMPLE_PROTOCOL__PACK_APP`. Declare only the hash variables a parent task consumes in its Turbo `env` list. Use an uncached parent preparation task that invokes `sources task` for required child outputs, then depend on that preparation task. Turbo owns the graph and cache; artifact staging and pnpm file-dependency refresh remain explicit workflow steps. Install local Turbo in the parent too.

The outer lock is released before parent Turbo starts so nested task calls work. Keep configuration, checkout selection, and source contents stable during that parent build. Inherited `SOURCES_` values are cleared from all Turbo invocations. Parent Turbo receives its complete fresh snapshot; a configured child receives its own selected graph's relevant source hashes consistently for hash queries and execution. See [the bridge contract](docs/turbo-integration-spec.md) for a complete configuration example.

For reuse based on emitted bytes, opt into per-task `content` artifact declarations.
Sources runs the required upstream Turbo task, refreshes declared installed
counterparts with the consuming owner's `pnpm install --frozen-lockfile`, and
verifies their bytes before downstream hashing. Declare the separate
`SOURCES_<PACKAGE>__<TASK>__CONTENT` env input in downstream Turbo. Existing task
hash variables retain their meaning. The [content preparation contract](docs/turbo-integration-spec.md#opt-in-consumed-artifact-content-preparation)
covers required metadata/assets, direct checkout consumption, native filtered
refresh, locking and migration.

## Commands

| Command | Behavior |
| --- | --- |
| `update` | Install the latest stable CLI release on Linux/macOS and print the skills update command. |
| `skills [name]` | List bundled agent skills or print the complete named skill without installation. |
| `turbo [args ...]` | Run parent Turbo with all configured child task hash variables. |
| `task package:task...` | Run allowed exact child Turbo tasks in active checkouts. |
| `setup` | Create missing configuration, directories, and ignore entries. |
| `check` | Validate all development mappings and repository identities locally. |
| `dev` | Select validated development checkouts; install configured pnpm file dependencies. |
| `dev-hints [name] --ref branch\|tag\|commit` / `--hint <git-ref>` | Update advisory development refs in `.sources.conf` for one or all sources. |
| `lock [--git] [name:commit or name:tag=tag ...] [--check]` | Capture development HEADs or update selected commit/tag pins atomically. `--check` caches inputs and selects named tags from upstream. |
| `lock --registry package:version... [--check]` | Pin exact package versions; `--check` downloads releases before writing. |
| `fetch [name ...]` | Contact selected upstreams and cache locked revisions, or default-branch tips for unpinned sources. Preserve existing checkout HEADs and links. |
| `check-release` | Cache complete Git release inputs without activating them; registry readiness is not checked. |
| `release [--git]` | Activate pinned Git checkouts through stable paths. |
| `release --registry` | Select locked package versions, install with pnpm, and activate other sources through Git. |
| `release --clean` | Recreate disposable managed checkouts from upstream and activate the lock. |
| `release --offline` | Activate complete cached inputs with no tool-initiated network access. Cannot combine with `--clean`. |
| `status [--json]` | Inspect links, mode, commits, changes, and local cache readiness. JSON has a versioned top-level schema. |
| `run name -- command [args ...]` | Execute directly in the named active source, inherit standard streams, and report changes before and after. |

Use `sources --help` or `sources <command> --help` for syntax. Results go to stdout; warnings and errors go to stderr. Exit codes are `0` for success, `1` for operational or validation failure, and `2` for invalid syntax. Once a child starts, `run` preserves its exit status. Shell expressions require an explicit shell, such as `sources run library -- sh -c 'build && test'`.

`check`, `status`, and ordinary `lock` do not fetch. `dev` never fetches Git repositories; opted-in pnpm installs may access the package registry. `run` does not fetch itself, though its child may use the network. Online release and readiness commands can reuse cached objects. `fetch` contacts every selected upstream even when cached.

## Fetching and ownership

Managed acquisition prefers depth-one fetches of required commits with complete file contents. It recursively acquires submodules at recorded gitlink commits. Sources does not use partial clones, blob filtering, sparse checkouts, moving submodule branches, or `git pull`. If a server requires broader history, Sources reports the compatibility fallback. Development commands inspect existing submodules without updating them.

A commit in a valid managed cache with the expected upstream is enough for pin validation. This does not prove that the remote still serves the commit, approve the source, or establish that tests passed. A clean release obtains inputs from upstream again. Offline release needs the complete parent and recursive submodule inputs already cached.

Everything under `.sources/` is disposable. Keep valuable work in development checkouts elsewhere. Links directly inside the configured workspace link directory are replaceable, including stale source links. Sources removes link objects without deleting their targets. It refuses ordinary files or directories at required link locations. Linux and macOS use directory symlinks; Windows may use junctions.

Sources never resets, stashes, cleans, switches branches in, or deletes development checkouts. Normal release activation refuses managed source edits and checkout conflicts. It preserves ignored artifacts unless they conflict with the requested checkout.

## Repair interrupted work

Multi-source activation is not atomic. An interruption may leave partial checkouts or links. Sources reports obvious breakage on the next relevant command and suggests a repair:

- `sources dev` restores development links.
- `sources fetch` caches missing inputs.
- `sources release` reapplies pins when managed clones remain usable.
- `sources release --clean` recreates broken managed contents, discarding their files.

There is no rollback journal or saved mode flag. `status` derives `dev`, `release`, `mixed`, or `unconfigured` from actual links and opt-in pnpm override entries. Links in release mode can still have commit or submodule mismatches, which status reports separately.

All project commands take one exclusive advisory lock at `.sources/project.lock`. `turbo` releases the lock after its hash snapshot, before parent execution. `task` waits for a busy lock to serialize child Turbo operations; other commands fail immediately. On Unix, `run` with an accessible controlling terminal keeps the shell's process group, including in pipelines, with redirected streams, or in background jobs. The shell controls terminal ownership and job suspension. Sources holds the lock until its direct child exits. Terminal interrupts reach the shell job group naturally; Sources absorbs its own interactive `SIGINT`, so programmatic interruption must target the job group. A `SIGTERM` sent explicitly to Sources is forwarded to its direct child. Interactive commands must wait for their own workers; workers left behind afterward are outside the lock and supervision guarantee.

Without a controlling terminal, `run` uses a dedicated child process group, forwards interruption and termination to it, and holds the lock until that group finishes. Jobs that leave the group are outside supervision. Process termination releases the lock; do not remove the lock file or `.sources/` while an operation is running.

## Development

The acceptance tests invoke the public CLI with real temporary Git repositories, filesystem links, and child processes. They isolate Git configuration and use loopback Git and HTTP fixtures. Python 3 is needed for the HTTP and npm registry fixtures. Node 24.22.1 and pnpm 10.15.0 exercise actual package installations against the loopback registry. Turbo tests also require the real Turbo 2.8.0 npm distribution. Install it outside this checkout with `npm install --prefix /tmp/sources-turbo-tools turbo@2.8.0`, then set `SOURCES_TEST_TURBO` to its platform native executable, for example `/tmp/sources-turbo-tools/node_modules/turbo-linux-64/bin/turbo` on Linux x86-64. macOS uses `turbo-darwin-arm64`; Windows uses `turbo-windows-64/bin/turbo.exe`. Keep the sibling `turbo` package installed too. Acceptance fixtures copy this installed tool and bootstrap dependency-free workspaces offline; `cargo test` does not download Turbo. Failed test fixtures remain on disk, with their paths printed for inspection.

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

CI builds checksum-pinned Git 2.56.0 on Linux and macOS and also tests Git 2.39.5 on Linux. Native macOS tests run on Apple silicon. Windows CI uses the runner's Git and is allowed to fail. Separate release-candidate jobs build archives and test extracted executables and installers. Rust and Cargo dependencies are pinned by `rust-toolchain.toml` and `Cargo.lock`; hosted runner system libraries still come from their OS images.

## Publish a Sources release

Update the Cargo package version and lockfile, commit the change, and manually publish a GitHub Release with the matching tag, such as `v0.1.0`. Publishing from the GitHub UI or `gh release create` triggers the release workflow, including when publishing a saved draft or prerelease. Saving an unpublished draft does not trigger a build.

The workflow checks out that exact tag, checks its version, builds native Linux x86-64, Apple silicon macOS, and Windows x86-64 archives, and tests packaged executables and installers. After all three platform builds succeed, it uploads the archives, checksums, and generated shell/PowerShell installers to the existing release. Wait for the workflow to finish before sharing installation commands. The workflow does not create releases or push tags; normal pushes never publish assets.

Pull requests and manual candidate runs verify distribution without uploading GitHub Release assets. cargo-dist 0.33.0, Rust, and Cargo dependencies are pinned. Windows runtime support remains best effort, while every advertised release archive must build and pass its installation checks.

Sources is available under the [MIT license](LICENSE).

See [the specification](docs/sources-cli-spec.md) for the full behavior contract and [.agents/refactor-policy.md](.agents/refactor-policy.md) for the compatibility policy.
