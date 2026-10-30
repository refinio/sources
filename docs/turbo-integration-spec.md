# Sources–Turbo integration specification

Status: implemented bridge contract, with opt-in content preparation added in October 2026. Read alongside `docs/sources-cli-spec.md`; this proposal extends its command and locking contracts.

## Purpose

Bridge independent Turbo-managed repositories without combining their pnpm workspaces or implementing another build system in Sources.

Sources selects checkouts, restricts the child Turbo entry points the parent may invoke, and passes child task hashes to the parent's Turbo environment. Turbo owns task definitions, prerequisites, scheduling, input hashing, outputs, execution, and caching inside each repository.

The baseline below describes task-hash mode. The [content preparation extension](#opt-in-consumed-artifact-content-preparation) explicitly adds upstream execution, fingerprinting, native frozen install refresh, and installed-byte verification for opted-in tasks.

All feature configuration belongs in the consuming parent's `.sources.conf`. Child repositories need normal working Turbo tasks, but no Sources-specific configuration or metadata. No generic command arrays, arbitrary shell commands, or direct `package.json` script execution are supported by these new commands. Turbo naturally executes its underlying scripts as usual.

## Agreed configuration

Add an optional per-source `packages` map. Each key is the actual child package name used by Turbo. Each package fully declares its own allowed tasks; there is no shared task-definition section and no separate user-defined hash key.

```json
{
  "version": 1,
  "directory": "./sources",
  "sources": {
    "shared": {
      "url": "https://github.com/example/shared.git",
      "packages": {
        "@example/protocol": {
          "tasks": ["build", "pack"]
        },
        "@example/ui": {
          "tasks": ["pack"]
        }
      }
    }
  }
}
```

The example assumes an additive extension to version 1; follow the repo's experimental compatibility policy if implementation requires a format change. Package paths remain in the `pnpm` mappings; exact versions now live beside Git pins in lock format 2. Explicit `release --git` and `release --registry` select the desired input. Sources without `packages` continue to work.

Validation:

- A package must resolve unambiguously to one configured source. Reject duplicate package names across sources rather than choose the first.
- Task lists contain distinct nonempty Turbo task names. Reject missing/empty/malformed configuration with actionable errors.
- Reject package names containing `__`; that delimiter is reserved for the package/task hash identity.
- Validate normalized environment-name uniqueness across every configured package/task pair before execution.
- Reject unknown fields consistently with the existing configuration model; generic `command` or `script` fields are not an escape hatch.

## Command: `sources turbo`

Syntax:

```sh
sources turbo run build
sources turbo run build --filter=@app/web
sources -C /path/to/parent turbo run build --filter=@app/cli
```

This is a wrapper around the parent's installed Turbo executable. It accepts and forwards normal Turbo arguments; it does not introduce another parent task syntax.

Behavior:

1. Discover the parent Sources project and query its locally installed Turbo with
   the original task/filter/selection options and `--dry=json`. The configuration
   is an allowlist of available foreign entry points, not a list to execute/query.
2. Supply candidate managed variable names with empty placeholders only for native
   graph discovery. Read the selected graph's configured hashed environment inputs,
   including native prerequisites, global env, wildcards and exclusions. Passthrough
   variables alone do not create required foreign edges.
3. Follow only those foreign inputs into their active owners. Batch compatible
   queries per canonical checkout/frontier, and reuse native metadata for task
   closures already discovered. Track edges by canonical checkout plus task, so an
   independent task in the parent's own repository can be a required producer.
4. Complete discovery and diagnose genuine task cycles before execution. Discovery
   hashes are provisional. Settle required foreign snapshots bottom up, querying
   final native hashes in owner/frontier batches after required content producers
   and installed-payload refresh have completed. Each entry snapshot is memoized
   within the invocation; no persistent graph/hash cache is introduced.
5. Hash inputs retain native task-hash values. CONTENT inputs require native
   producer execution/cache restoration and consumed-artifact fingerprinting.
   Existing native task prerequisites and explicit staging still run normally.
6. Run the parent Turbo with the original arguments and only its required managed
   input values. Parent native scheduling, output caches and CLI behavior remain
   owned by Turbo. In dry-run mode, preparation output goes to stderr so stdout
   remains the requested native dry-run result.

Only required active checkouts need to be available. No implicit fetch, checkout
selection, tool installation or pin changes occur. Content declarations explicitly
authorize the native producer preparation and frozen pnpm refresh described below.

Use the installed Turbo belonging to each repository, not an accidentally inherited parent version. `pnpm exec turbo` in the relevant repository is the initial runner consistent with current project tooling. Do not auto-download a global/latest Turbo or execute arbitrary package scripts to imitate it. Missing bootstrap/tooling is a clear prerequisite error.

## Per-task environment variables

Identity: `(package name, task name)`.

Naming: `SOURCES_<NORMALIZED_PACKAGE>__<NORMALIZED_TASK>`.

Normalize each component by uppercasing ASCII letters, replacing each maximal run of non-ASCII-alphanumeric characters with `_`, and removing leading/trailing `_`. Empty components are invalid. For example:

```text
@example/protocol : build     → SOURCES_EXAMPLE_PROTOCOL__BUILD
@example/protocol : pack      → SOURCES_EXAMPLE_PROTOCOL__PACK
@example/protocol : pack:app  → SOURCES_EXAMPLE_PROTOCOL__PACK_APP
@example/ui       : pack      → SOURCES_EXAMPLE_UI__PACK
```

The raw package name cannot contain `__`. Reject normalization collisions, including package punctuation/case differences and task names such as `pack-app` versus `pack:app`. Do not resolve collisions with unstable numbering or silently overwrite a value.

The value is the selected task's Turbo hash, unchanged. Each required task gets a separate variable; do not collapse all tasks in a package/repository into one hash. Avoid run IDs, timestamps, cache hit/miss status, progress/log text, or output enumeration order in the value.

`SOURCES_` is the managed namespace for this bridge. Computed values override inherited values; stale inherited hash variables from a prior configuration must not enter parent hashing as current values. Do not mutate the invoking shell's environment. Preserve ordinary unrelated environment.

A Turbo task hash is an input-derived cache key, not a cryptographic checksum of artifact bytes. Correctness depends on the child's declared inputs, task dependencies, relevant environment/toolchain, and deterministic outputs. Sources does not repair an incorrect Turbo graph or promise content integrity from its cache key.

## Composing direct edges across repositories

A child may itself be a Sources project. Keep each consuming repository configured for its direct dependencies. When C consumes B and B consumes A, B's Turbo task declares A's managed hash in `env`, and C declares only B's hash. Changes in A then change B's actual Turbo hash and propagate to C. No ancestor needs to duplicate the transitive configuration.

Required owner/task closures are discovered before preparation. Native queries are
batched per owner/frontier, retaining task-local dependency edges from the native
metadata rather than combining unrelated allowlisted tasks. Final hashes are
collected after the required upstream content and refresh stages, then memoized
for that invocation. Child-owned environments are used consistently for final
queries, native execution and refresh lifecycles.

Cycles are task cycles, not repository cycles. VGER application -> AKTE module ->
a different VGER IMAP task is valid when that task's required closure is acyclic.
A true task cycle fails with its canonical checkout/task path before execution.

Project locks cover configuration/selection inspection, metadata queries and
installed refresh phases. They never span another owner's traversal or native
Turbo execution. This permits a native task to invoke `sources task` in another
owner and return to an independent task here without project-lock re-entry.
Concurrent metadata/refresh phases serialize on the owner lock; native task
lifetimes are not globally serialized by Sources. Keep selections, configuration
and build inputs stable for the whole invocation. `sources run` retains its
separate whole-command lock contract.

Hash composition requires active checkouts and installed local Turbo. It does not bootstrap nested Sources projects. `sources release` activates only its own project's direct pinned Git checkouts and their Git submodules; it does not recursively interpret the checkouts' `.sources.conf` files or share an ancestor's checkout selections.

A finite selected package graph can use native filtered frozen pnpm installs even when the repository graph contains a cycle. Select the private tooling root and the exact required packages with their dependency closures. For example, an application in ONE may consume Fotos core, which consumes ONE foundation. Activate the needed source edges explicitly, then install and build from foundation toward the application. The following fixture names illustrate the sequence; substitute the actual root and package names from each owning repository:

```sh
sources release
sources -C sources/fotos release

pnpm --dir sources/fotos/sources/one --filter one-tooling --filter '@fixture/one-foundation...' install --frozen-lockfile
sources -C sources/fotos task @fixture/one-foundation:build

pnpm --dir sources/fotos --filter fotos-tooling --filter '@fixture/fotos-core...' install --frozen-lockfile
sources task @fixture/fotos-core:build

pnpm --filter one-tooling --filter '@fixture/one-app...' install --frozen-lockfile
sources turbo run build --filter=@fixture/one-app
```

For several independent foundation packages, list each required `--filter '<package>...'`; one selector suffices only when its dependency closure includes them all. Include the owning private root so its pinned Turbo and other build tools are installed. `--offline` is optional and requires the package store to be populated. The examples use Git sources without registry-selection mappings, so activation and pnpm installation remain separate explicit steps.

The foundation checkout's unselected application may reference `sources/fotos` without activating that edge. Its own Sources configuration remains present, but the selected foundation tasks must not consume that external hash or require that directory. The two ONE checkouts may have different commits: each selected checkout must honor the Git pin of its immediate consuming project. Do not substitute an ancestor checkout for a different child pin.

This workflow was verified with pnpm 10.15.0, real Git release checkouts, committed frozen locks, and Turbo 2.7.6 installed by pnpm. A disposable workspace with an unselected missing directory-file dependency installed its tooling root and selected package dependency closure with a fresh store, leaving the unselected importer uninstalled and the lock unchanged. Whole-workspace frozen installation failed on the missing target. A separate ONE/Fotos release fixture then completed the directed build above while the foundation owner's nested Fotos link stayed absent; both selected managed commits matched their Sources pins and all pnpm locks stayed unchanged. Ordinary lifecycle-script handling was used in that release fixture.

This proves the selected-closure workflow, not recursive activation of every embedded Sources project or whole-workspace installation. Actual repository filters, frozen locks, internal file dependencies, tooling-root dependencies, and lifecycle scripts must respect that closure. Build each source output before installing the downstream directory-file package that snapshots it. If a required selected importer or lifecycle script needs the missing reverse edge, filtering does not solve that graph; assess that separately instead of adding unbounded recursive activation. The actual consuming repositories still require their own integration validation.

## Command: `sources task`

Syntax:

```sh
sources task @example/protocol:pack
sources task @example/protocol:build @example/protocol:pack @example/ui:pack
sources task @example/protocol:pack:app
```

Accept one or more selectors. Parse the package/task separator at the first colon; preserve subsequent colons in the task name. Package names are actual package identifiers, not source directory names.

Behavior:

1. Validate all selectors, owning sources, active checkouts, normalized identities, and selected child Turbo tasks before executing any selected task.
2. Only packages and tasks in the parent's configured allowlists may be requested directly. Dependencies that Turbo itself schedules need not be separately exposed as Sources entry points.
3. Resolve each selector to its selected source checkout and invoke that repo's installed Turbo. Never call `pnpm run <task>`/`npm run <task>` directly, execute a raw command from config, or use an implicit shell.
4. Accept multiple tasks, including multiple packages and multiple sources. Exact duplicate selectors may be deduplicated while preserving the first occurrence.
5. Preserve exact pairs. For example, `protocol:build ui:pack` must not become `turbo run build pack --filter=protocol --filter=ui`, which can execute unintended pairs. Use supported package-qualified Turbo task identifiers, such as `<package>#<task>`, or separate exact invocations when necessary.
6. Within a source, let Turbo determine ordering and caching from its graph. CLI argument order does not add dependency edges. Grouping requests is allowed when it preserves selection semantics. Across sources, use deterministic sequential groups and stop at the first failure; do not invent cross-repository dependency scheduling.
7. Forward child output and preserve the nonzero exit status when execution fails. Do not roll back successful earlier task outputs. Apply existing dirty-source warnings before/after execution.

This command does not fetch or select another mode. If the chosen source is currently represented by a registry release with no active checkout, fail clearly instead of using an unrelated development checkout. Registry-only hash/task semantics are out of scope; preserve the current published-package integration.

## Parent usage and cache granularity

Parent application `package.json`:

```json
{
  "scripts": {
    "prepare:protocol": "sources task @example/protocol:pack",
    "build": "vite build"
  }
}
```

Parent `turbo.json`:

```json
{
  "tasks": {
    "prepare:protocol": {
      "cache": false
    },
    "build": {
      "dependsOn": ["prepare:protocol"],
      "env": ["SOURCES_EXAMPLE_PROTOCOL__PACK"],
      "outputs": ["dist/**"]
    }
  }
}
```

Launch the parent with `sources turbo run build`. The uncached preparation bridge always invokes child Turbo; the child owns its build/artifact cache. Parent tasks declare only the external task hashes they consume. Avoid putting all external hashes in `globalEnv` when independent builds need separate cache invalidation.

The consumer still needs an explicit artifact staging/install-refresh step where its file dependencies require it. The example illustrates task/hash wiring, not a complete installer. Parent script/task names and outputs are project-specific; avoid registering a wrapper root task that recursively invokes itself.

## Artifacts and non-goals

Get declared output paths and hashes from Turbo; do not duplicate artifact paths in `.sources.conf`. Turbo `--dry=json` exposes declared output globs and task hashes. A completed `--summarize` run exposes resolved output files, including cache hits. Filter selected task outputs appropriately; cached Turbo log files are not package artifacts.

Packaging tasks must produce correct installable outputs under their own declared contract. No arbitrary output is automatically an npm package. Artifact extraction, staging, checksum calculation, installation refresh, and a new artifact-management API are not part of this first hash/task bridge unless separately agreed. Sources remains usable without Turbo when these optional features are not configured.

No child Sources metadata, child source-tree hashing in Sources, shared generic task registry, generic script runner, custom cache, build-output repair, task graph/scheduler, automatic workspace merging, or publishing is introduced.

## Locking and process supervision

`sources turbo` and `sources task` hold the current owner's project lock during
configuration/selection inspection, native metadata queries and installed refresh.
They release it before traversing other owners or executing native Turbo. Brief
metadata/refresh contention waits on that same OS lock; no lock is held across a
foreign traversal or a native task that may return to another task in this owner.

This intentionally replaces the previous whole-lifetime `sources task` lock and
serialization contract. Keep source selection, configuration and inputs stable
throughout a build. Native Turbo owns scheduling/caching within each repository.
Other Sources commands, including `run`, retain their existing lock behavior.
Process supervision and exit-status/signal propagation remain unchanged.

## Verification evidence and implementation acceptance

Prior disposable checks with real Turbo 2.7.6 and pnpm 9.15.4 established:

- `--dry=json` contains selected task IDs, hashes, dependencies, commands, and declared output patterns. `expandedOutputs` was empty in dry-run metadata.
- A cache-hit `--summarize` response contained the task hash and concrete archive output path, plus an internal Turbo log path that must not be mistaken for an artifact.
- Child-owned build/package caching and a parent hashed environment input correctly handled unchanged sources, dirty edits, added/deleted source files, packaging-only README changes, and restoration after deleting generated/install outputs.
- Restoring cached unpacked directories can leave stale files. This is a child/output contract concern, not a reason to add hidden cleanup to this bridge.

Implement and verify through the repository's public CLI acceptance-test seam. Required cases:

1. Sources with package paths in `pnpm` mappings and lock format 2 work with both Git and registry selection; new commands appear in help.
2. Valid configuration, scoped package names, task names with colons, multiple configured tasks, and multiple sources.
3. Invalid/empty entries, duplicate package owners, forbidden `__`, and normalized environment collisions fail clearly.
4. Missing task/package or unconfigured selector fails before any selected execution; generic script/command fallback never occurs.
5. Multiple exact task selectors execute only their requested pairs; Turbo prerequisites remain allowed.
6. Child dry-run metadata yields separate deterministic env values; cache-hit metadata does not change values. Inherited stale values cannot override the snapshot.
7. Parent arguments, unrelated environment, output, child errors, and nonzero statuses are forwarded correctly; failed metadata collection never launches the parent.
8. Parent runs can invoke one or multiple concurrent `sources task` prerequisites without outer-lock deadlock or random busy failures.
9. Missing/mismatched active checkouts, managed dirty warnings, registry-only selections, missing local Turbo/bootstrap, malformed JSON, and unsupported metadata shapes fail with useful diagnostics.
10. A real-Turbo smoke proves per-task granularity: changing one child task input changes its corresponding exported key and invalidates only parent tasks declaring that key, subject to legitimate Turbo upstream dependencies.

Follow `AGENTS.md` and local compatibility/test/review policy. Required repository checks include formatting, clippy, and the acceptance suite. Do not push main branches, merge, publish releases, or modify unrelated consumer repositories for this handoff.

## Primary references

- https://turborepo.dev/docs/reference/run#--dry---dry-run
- https://turborepo.dev/docs/reference/run#--summarize
- https://turborepo.dev/docs/reference/run#using-a-task-identifier
- https://turborepo.dev/docs/crafting-your-repository/using-environment-variables

## Opt-in consumed-artifact content preparation

The October 2026 extension adds an explicit `content` map beside a package's
existing `tasks` allowlist. Existing task-hash variables retain their values and
semantics. No configuration or lock version changes, Turbo upgrade, consumer
migration, or dependency installation occurs for configurations without `content`.

```json
{
  "packages": {
    "@example/protocol": {
      "tasks": ["pack"],
      "content": {
        "pack": {
          "artifacts": [
            {"path": "packages/protocol/dist", "installed": "node_modules/@example/protocol/dist"},
            {"path": "packages/protocol/package.json", "installed": "node_modules/@example/protocol/package.json"},
            {"path": "packages/protocol/assets", "installed": "node_modules/@example/protocol/assets"}
          ]
        }
      }
    }
  }
}
```

This is a fragment of one source's configuration. Each content key must be an
allowed task. `artifacts` is a nonempty list of required files or nonempty directory
trees. `path` is relative to the selected upstream checkout. Optional `installed`
is the corresponding file or directory relative to the consuming project, after
native installation. Paths are literal nonempty relative paths without `.` or `..`
components. Declare every consumed payload, including declarations, metadata and
assets. Use individual file declarations when those files must exist even if their
containing directory still has other files. Omit `installed` for bytes consumed
directly through a managed Sources checkout link.

Turbo remains responsible for declaring and restoring its outputs. These consumer
paths select the actual consumed subset, which can also include static package
metadata. They do not replace Turbo `outputs` or npm `files`/exports declarations.
The packaging/staging task must prepare the exact installable contract before
returning. Sources does not copy files, generate package metadata, clean outputs,
interpret exports, or synthesize package archives.

For `sources turbo`, Sources now performs these steps for configured content tasks:

1. Resolve child-owned managed environment inputs with native Turbo dry metadata,
   including global env, transitive tasks, wildcards and exclusions. Resolve required
   upstream edges recursively and diagnose task cycles.
2. Run each required upstream task once per canonical checkout/task within the
   invocation, using that repository's native Turbo and resolved environment. Turbo
   schedules its own prerequisites and restores its caches. Stop on failure and
   preserve the upstream Turbo exit code. Release phase locks before native
   execution, including when the producer is an independent task in the caller's
   repository.
3. Fingerprint required consumed artifact paths after successful execution or cache
   restoration. Missing paths and empty required trees fail before downstream Turbo.
4. When installed counterparts are declared, run the consuming repository's native
   `pnpm install --frozen-lockfile`, once after its upstream group is prepared. Require
   an existing ordinary `pnpm-lock.yaml` and an exact `packageManager: pnpm@<version>`
   in its root manifest. The pnpm dispatcher must report that declared version.
   Use the owner's normal registry, store, workspace, and lifecycle configuration.
   Install lifecycles receive this consuming checkout's fresh managed snapshot;
   remove inherited stale/ancestor `SOURCES_*` values just as for native Turbo.
   Do not add `--ignore-scripts`, `--force`, update flags, or disable lifecycle scripts.
   Frozen resolution preserves recorded versions and fails on lock/manifest drift.
   By default installation covers the normal workspace. An optional root
   `.sources.conf` field `"pnpm_refresh":{"filters":["tooling-root","@app/web..."]}`
   selects the owner's existing finite importer closure through literal native
   `--filter=<selector>` arguments. Include its tooling root and every consumed
   importer that needs installation. Sources does not invent or widen selectors;
   installed-artifact verification rejects incomplete refreshes.
   Resolve all selected tasks' consumed edges before refreshing a checkout with
   their combined environment. Collect final native hashes only after that shared
   refresh, so a lifecycle generator cannot erase an earlier task's inputs by
   seeing only a later task's partial environment. Memoized tasks retain their own
   consumed edges, preserving task-level cycle handling. If a selected task's hash
   was already resolved and the shared refresh changes it, fail and require a new
   invocation rather than replacing a snapshot already used by another consumer.
   Sources does not monitor lifecycle mutations of unselected task inputs. Keep
   lifecycle generation and explicit staging consistent with the selected contract;
   after editing those inputs or selections, rerun the complete command.
   This opt-in refresh is independent of the selection-time install skip logic.
5. Compare installed consumed trees with the prepared upstream trees, including
   relative file names, file bytes and Unix executable bits. Recheck upstream bytes
   after installation as well. Missing, stale, extra, or lifecycle-modified installed
   files fail preparation. No downstream hash or task execution is accepted on that
   failure. pnpm package-directory links, including an external configured virtual
   store, are resolved at each declared root. Descendant links/special files are
   unsupported and fail explicitly; declare ordinary staged payloads instead.
6. Supply `SOURCES_EXAMPLE_PROTOCOL__PACK__CONTENT` as a separate declared Turbo env
   input. Its `sha256:` value covers a versioned, length-delimited, sorted inventory
   of declared upstream paths, relative file names, bytes and executable bits.
   It excludes absolute checkout paths, timestamps, native task hashes, logs, cache
   status and run IDs. Refresh and verification precede the consuming Turbo's actual
   hashing, including at intermediate repositories.

The existing `SOURCES_EXAMPLE_PROTOCOL__PACK` still contains Turbo's input-derived
hash. For content-based reuse, downstream `env` should select only the
`__CONTENT` variable. A broad `SOURCES_*` wildcard includes both values unless it
explicitly excludes the task-hash variable. Keeping the original task hash as an
additional hashed input deliberately retains source-derived invalidation.

Pin consuming repositories to Turbo 2.8.0 for this content workflow. Preparation
happens before downstream invocation and requires no deferred Turbo hashing.
Sources uses each repository's locally installed Turbo and does not install or
upgrade it. The maintained acceptance workflow pins Turbo 2.8.0 and pnpm 10.15.0;
older-version compatibility is outside this feature's scope. Sources does not
claim to accelerate real consumers based on the disposable fixture results.

`sources task` validates every explicitly requested executable task and its
required closure before execution. It uses the same selected graph and canonical
owner/frontier batching. Direct tasks run through native Turbo; CONTENT inputs
are prepared first, while native task-hash inputs preserve the existing explicit
staging contract. No allowed but unconsumed task becomes a required dependency.

Both Turbo commands use phase-local project locks. Refreshes serialize, but native
execution runs without a project lock so valid repository return edges work.
Existing explicit staging tasks still run, and native caches decide whether they
execute or restore. Do not change checkouts/configuration during a build. A pnpm
lifecycle must not reenter a Sources command in its own locked refresh phase.
Installation is not transactional; fix an error and rerun. No rollback, arbitrary
command hook, registry-selection change, recursive activation or publishing is
introduced.

### Acceptance and review scope

The approved task is output-content-based caching across independent repositories,
with Sources-owned native frozen pnpm refresh and installed-artifact verification.
The public CLI stories use disposable real Git repositories, Sources-managed dev
links, a pinned managed checkout, and ordinary pnpm directory-file installation.
They reject stale downstream results and check native Turbo summaries for initial
MISS, unchanged/cache-restored HIT, upstream source MISS with identical bytes and
downstream HIT, changed content/path/deletion MISS, and asset invalidation. Upstream
failure, missing required output, frozen-lock drift, and lifecycle-corrupted
installed payloads must prevent downstream execution. Recursive content edges
retain child env ownership, wildcard exclusions, task-cycle diagnostics and the
valid return-to-caller producers, filtered allowlists and batched closures. Existing native task-hash and nested-task stories remain
required. No live consumer, global tool, release asset or RoboRev configuration is
part of this change.
