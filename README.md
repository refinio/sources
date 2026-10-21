# Sources

`sources` selects development checkouts or exact release commits through stable workspace paths. Git source workspaces keep the same paths when switching between them. Optional pnpm integration switches local file dependencies to published package versions. Sources invokes the installed Git executable and does not build packages or publish releases.

Linux and macOS are supported. Native Windows is best effort. Use Git 2.39.5 or newer, with the SSH and credential setup your repositories require. The CLI is experimental and currently promises no backwards compatibility.

## Install

Install the latest stable GitHub Release on Linux x86-64 or macOS Intel/Apple silicon:

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

## Agent skills

After a successful binary installation, the installer prints this optional next step:

```sh
npx skills add refinio/sources
```

Choose the skills and agents to install to. This command uses Node/npm and does not run automatically during CLI installation.

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
      "url": "https://github.com/your-org/library.git"
    }
  }
}
```

Replace the example URL. Each source name is one directory name. For example, `library` appears at `sources/library`. Point your Cargo, pnpm, or other workspace configuration at these stable paths.

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

The generated lock is JSON with `version: 1` and a `sources` object mapping source names to full commit object IDs. Review and commit it with the consuming app changes. A lock captures committed revisions only. Dirty development checkouts produce warnings; their uncommitted edits are excluded.

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

For a pnpm project, opt a source into registry releases by adding its package names, paths within the development checkout, and exact published versions:

```json
"library": {
  "url": "https://github.com/your-org/library.git",
  "pnpm": {
    "@your-org/core": {"path": "packages/core", "version": "1.2.3"},
    "@your-org/ui": {"path": "packages/ui", "version": "2.0.0"}
  }
}
```

Place this entry inside `.sources.conf`'s `sources` map. Require pnpm 10.15 or newer, a root `package.json`, and `pnpm-workspace.yaml`. Consumers keep ordinary dependencies by package name. Keep these external packages out of pnpm workspace globs, including broad globs over `sources/`. Move legacy `package.json` pnpm overrides into the workspace YAML first.

pnpm file paths must be valid Unicode and cannot contain literal Unix backslashes. These pnpm restrictions do not apply to Git-only sources.

```sh
sources dev       # file: overrides through local source links, then pnpm install
sources release   # exact registry overrides, then pnpm install
```

Sources manages the corresponding root `overrides` in `pnpm-workspace.yaml`, including references between managed packages. It preserves unrelated settings and comments. This selection applies throughout the dependency graph. Both commands run `pnpm install --no-frozen-lockfile` and may change `pnpm-lock.yaml`; consumer `package.json` dependency declarations stay intact. Review and commit release-mode workspace configuration and the pnpm lock together. CI can then verify them with `pnpm install --frozen-lockfile`.

Published-package sources do not need a Git pin, development mapping, or managed clone for release mode. Other sources in the same project can keep using exact Git commits. Set package release versions explicitly in `.sources.conf`; switching never selects `latest` or changes these versions. `lock`, `fetch`, and `check-release` manage Git inputs only.

`release --offline` uses pnpm's offline store and existing Git caches. `--clean` clears Sources state, leaving the app's `node_modules`, store, and outputs to pnpm and existing build tooling. pnpm installation follows the project's lifecycle-script settings. Its `file:` dependencies may need reinstalling after new files, atomic saves, or generated output changes; they do not guarantee live symlink updates.

`status` reports selected references, not installed-package verification. `sources run` requires a checkout, so use `sources dev` before running a command in a source currently selected as a registry release. If installation fails, fix the cause and rerun the intended Sources command.

## Commands

| Command | Behavior |
| --- | --- |
| `setup` | Create missing configuration, directories, and ignore entries. |
| `check` | Validate all development mappings and repository identities locally. |
| `dev` | Select validated development checkouts; install configured pnpm file dependencies. |
| `lock [name:commit or name:tag=tag ...] [--check]` | Capture development HEADs or update selected commit/tag pins atomically. `--check` caches inputs and selects named tags from upstream. |
| `fetch [name ...]` | Contact selected upstreams and cache locked revisions, or default-branch tips for unpinned sources. Preserve existing checkout HEADs and links. |
| `check-release` | Cache complete Git release inputs without activating them; registry readiness is not checked. |
| `release` | Activate pinned Git checkouts and install configured exact pnpm package releases. |
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

All project commands take one exclusive advisory lock at `.sources/project.lock`. A busy lock fails immediately. On Unix, `run` with an accessible controlling terminal keeps the shell's process group, including in pipelines, with redirected streams, or in background jobs. The shell controls terminal ownership and job suspension. Sources holds the lock until its direct child exits. Terminal interrupts reach the shell job group naturally; Sources absorbs its own interactive `SIGINT`, so programmatic interruption must target the job group. A `SIGTERM` sent explicitly to Sources is forwarded to its direct child. Interactive commands must wait for their own workers; workers left behind afterward are outside the lock and supervision guarantee.

Without a controlling terminal, `run` uses a dedicated child process group, forwards interruption and termination to it, and holds the lock until that group finishes. Jobs that leave the group are outside supervision. Process termination releases the lock; do not remove the lock file or `.sources/` while an operation is running.

## Development

The acceptance tests invoke the public CLI with real temporary Git repositories, filesystem links, and child processes. They isolate Git configuration and use loopback Git and HTTP fixtures. Python 3 is needed for the HTTP and npm registry fixtures. Node 24.22.1 and pnpm 10.15.0 exercise actual package installations against the loopback registry. Failed test fixtures remain on disk, with their paths printed for inspection.

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

CI builds checksum-pinned Git 2.56.0 on Linux and macOS and also tests Git 2.39.5 on Linux. Native macOS tests cover Intel and Apple silicon. Windows CI uses the runner's Git and is allowed to fail. Separate release-candidate jobs build archives and test extracted executables and installers. Rust and Cargo dependencies are pinned by `rust-toolchain.toml` and `Cargo.lock`; hosted runner system libraries still come from their OS images.

## Publish a Sources release

Update the Cargo package version and lockfile, commit the change, and manually publish a GitHub Release with the matching tag, such as `v0.1.0`. Publishing from the GitHub UI or `gh release create` triggers the release workflow, including when publishing a saved draft or prerelease. Saving an unpublished draft does not trigger a build.

The workflow checks out that exact tag, checks its version, builds native Linux x86-64, macOS Intel/Apple silicon, and Windows x86-64 archives, and tests packaged executables and installers. After all four platform builds succeed, it uploads the archives, checksums, and generated shell/PowerShell installers to the existing release. Wait for the workflow to finish before sharing installation commands. The workflow does not create releases or push tags; normal pushes never publish assets.

Pull requests and manual candidate runs verify distribution without uploading GitHub Release assets. cargo-dist 0.33.0, Rust, and Cargo dependencies are pinned. Windows runtime support remains best effort, while every advertised release archive must build and pass its installation checks.

Sources is available under the [MIT license](LICENSE).

See [the specification](docs/sources-cli-spec.md) for the full behavior contract and [.agents/refactor-policy.md](.agents/refactor-policy.md) for the compatibility policy.
