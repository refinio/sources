# Sources–Turbo integration specification

Status: agreed feature proposal and implementation handoff, 2026-10-21. These commands are not implemented by this document. Read alongside `docs/sources-cli-spec.md`; this proposal extends its command and locking contracts.

## Purpose

Bridge independent Turbo-managed repositories without combining their pnpm workspaces or implementing another build system in Sources.

Sources selects checkouts, restricts the child Turbo entry points the parent may invoke, and passes child task hashes to the parent's Turbo environment. Turbo owns task definitions, prerequisites, scheduling, input hashing, outputs, execution, and caching inside each repository.

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

The example assumes an additive extension to version 1; follow the repo's experimental compatibility policy if implementation requires a format change. Do not silently remove the existing `pnpm` release-selection fields or change existing dev/release behavior. Sources without `packages` continue to work.

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

1. Discover and validate the parent Sources project using existing `-C`/ancestor-discovery semantics.
2. Resolve the currently selected active source checkouts, using the same identity/link validation and dirty-source diagnostics as existing execution commands. Do not fetch, switch modes, install tools/dependencies, change branches, or change pins implicitly.
3. Query each configured package/task pair through the child repository's installed Turbo, using machine-readable dry-run metadata, for example `turbo run ... --dry=json`. Queries may be grouped but must preserve exact package/task selection.
4. Require every explicitly configured pair to appear as an actual selected Turbo task; do not accept a silent no-op for a missing package or script/task. Normal dependency/transit tasks may also appear in Turbo's graph.
5. Read each selected task's `hash` from Turbo's response. Do not hash the source tree independently or substitute HEAD/mtime/package versions.
6. Construct the environment described below for all configured package/task pairs, regardless of the requested parent filter. Fine-grained parent task `env` declarations determine invalidation. Preparing/querying only a requested subset is a future optimization, not required now.
7. Finish the metadata snapshot and release the parent project lock before spawning the parent Turbo process. Preserve stdout/stderr, exit status, and interruption behavior consistently with the execution contract, subject to the explicit nested-command locking exception below.
8. Run the parent Turbo from the consuming project root with the forwarded arguments and computed environment.

Hash queries do not execute child build/pack tasks. The wrapper does not stage artifacts or refresh app dependencies. Those operations must be explicit prerequisites supplied by the parent workflow; this feature is the hash bridge, not a hidden package installer.

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

The value is the selected task's Turbo hash, unchanged. Every configured task gets a separate variable; do not collapse all tasks in a package/repository into one hash. Avoid run IDs, timestamps, cache hit/miss status, progress/log text, or output enumeration order in the value.

`SOURCES_` is the managed namespace for this bridge. Computed values override inherited values; stale inherited hash variables from a prior configuration must not enter parent hashing as current values. Do not mutate the invoking shell's environment. Preserve ordinary unrelated environment.

A Turbo task hash is an input-derived cache key, not a cryptographic checksum of artifact bytes. Correctness depends on the child's declared inputs, task dependencies, relevant environment/toolchain, and deterministic outputs. Sources does not repair an incorrect Turbo graph or promise content integrity from its cache key.

## Composing direct edges across repositories

A child may itself be a Sources project. Keep each consuming repository configured for its direct dependencies. When C consumes B and B consumes A, B's Turbo task declares A's managed hash in `env`, and C declares only B's hash. Changes in A then change B's actual Turbo hash and propagate to C. No ancestor needs to duplicate the transitive configuration.

For each selected child task, Sources first uses Turbo dry-run metadata to discover the managed environment inputs consumed by that task and its transitive Turbo prerequisites. A discovery query supplies the child project's configured variable names with empty placeholder values so Turbo itself expands environment patterns and exclusions. Sources resolves only the consumed configured package/task pairs, then repeats the query with their actual hashes. Global hashed environment declarations also participate. Placeholders never enter exported hashes or task execution.

Both the final metadata query and `sources task` execution use the child's computed snapshot. All inherited `SOURCES_*` values are removed first. Child repositories without `.sources.conf` at their root retain ordinary Turbo behavior with the managed namespace cleared. Missing or unused external environment names do not authorize implicit sources or task selection.

Resolution follows checkout and selected task identities, not repository edges alone. For example, ONE's application task may consume Fotos.core, which consumes ONE's independent foundation task. This repository cycle works when the selected task hash graph is acyclic. A genuine cycle fails before execution, reports the checkout/task path, and asks the caller to remove the cyclic `SOURCES_*` task environment dependency. Independent selected tasks are resolved separately, and shared task snapshots are reused within one CLI invocation.

Hash composition preserves the existing execution lock contract. `sources task` holds the invoking project's lock while child Turbo runs. Prerequisite scripts must not reenter a project lock held by an ancestor process. Such nested project operations remain unsupported, as specified in the core CLI contract. An acyclic hash graph alone does not authorize arbitrary recursive command execution.

The public CLI regression suite covers the natural three-repository chain, leaf edits, downstream hash changes, unchanged cache hits, and restoration after all build outputs are deleted. The suite also covers task granularity, transitive task environment declarations, wildcard exclusions, global environment inputs, repository cycles through independent tasks, and genuine task hash cycles. These guarantees preserve Turbo's ownership of hashes, scheduling, outputs, and caches. This change does not add output staging, package snapshot refresh, or dependency installation.

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

Current Sources holds `.sources/project.lock` across every command, including `run`. Holding that same lock throughout `sources turbo` would deadlock/fail when parent tasks invoke `sources task` in the same project.

Required behavior:

- `sources turbo` validates/resolves/queries its metadata snapshot under the project lock, then releases it before launching parent Turbo. Its process-waiting helper must not reacquire/retain it across the parent lifetime.
- Nested `sources task` calls acquire the normal project lock and use the existing child-process supervision guarantees for actual child execution.
- The same single build may include several parent preparation tasks. Existing immediate busy-lock failure must not make normal concurrent parent tasks randomly fail: serialize these task-command invocations through a documented blocking task lock/acquisition policy, or demonstrate an equivalent supported strategy. Preserve existing lock behavior for unrelated commands unless intentionally documented otherwise. Do not add generic rollback/reentrant-lock machinery.
- Concurrent checkout selection/configuration changes during a parent build are not promised safe. Document the stable-selection assumption; do not claim the released lock snapshots mutable checkout contents.
- Cover interactive/non-interactive child failure and interruption consistently with current Linux/macOS support and best-effort Windows behavior.

Update the base CLI spec's universal lock wording to describe this deliberate exception. Do not silently alter all existing execution commands to hold no lock.

## Verification evidence and implementation acceptance

Prior disposable checks with real Turbo 2.7.6 and pnpm 9.15.4 established:

- `--dry=json` contains selected task IDs, hashes, dependencies, commands, and declared output patterns. `expandedOutputs` was empty in dry-run metadata.
- A cache-hit `--summarize` response contained the task hash and concrete archive output path, plus an internal Turbo log path that must not be mistaken for an artifact.
- Child-owned build/package caching and a parent hashed environment input correctly handled unchanged sources, dirty edits, added/deleted source files, packaging-only README changes, and restoration after deleting generated/install outputs.
- Restoring cached unpacked directories can leave stale files. This is a child/output contract concern, not a reason to add hidden cleanup to this bridge.

Implement and verify through the repository's public CLI acceptance-test seam. Required cases:

1. Existing commands/configuration including `pnpm` mappings still work; new commands appear in help.
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
