---
name: sources-setup
description: Adopts and configures Sources for external Git checkouts at stable workspace paths. Use when users request symlinks or external local file/path dependencies, when loose external checkout dependencies are discovered, or when configuring an existing Sources project. Excludes ordinary dependencies within one monorepo.
---

# Set up Sources

If adoption has not been requested and no `.sources.conf` exists, ask whether the user wants Sources before changing the project; wait for acceptance. Briefly explain that Sources selects local development checkouts or fixed release inputs. Git releases use stable links; optional pnpm integration selects published packages. Existing Sources projects need no adoption question.

1. Inspect the intended app root, external Git checkouts, dependency references, and existing configuration. Keep internal monorepo dependencies unchanged.
2. Require Git and an installed `sources` CLI; check `git --version` and `sources --version`. If missing, use the platform installer in the [official README](https://github.com/refinio/sources#readme).
3. Run `sources setup` at the app root. Commands discover the nearest parent `.sources.conf`; use `sources -C <app-root> ...` when needed. Preserve existing entries.
4. Edit the UTF-8 JSON configuration, replacing these example names, URLs, and paths with actual values.

Committed `.sources.conf`:

```json
{
  "version": 1,
  "directory": "./sources",
  "sources": {"lib": {"url": "https://github.com/example/lib.git"}}
}
```

Ignored `.sources.local.conf`:

```json
{"sources": {"lib": "../lib"}}
```

`directory` and relative local paths resolve from the app root. Each local path must identify a Git checkout root with a matching remote; a fork may match through its `upstream` remote. Preserve original checkouts and their edits.

For Git releases, point workspace/package references at `sources/lib` or a package below it and keep these paths in both modes. For pnpm registry releases, add source `pnpm` mappings such as `"@example/ui": {"path":"packages/ui","version":"1.2.3"}`, keep consumers' ordinary package dependencies, and exclude mapped source packages from workspace membership. Require pnpm 10.15+, root `package.json` and `pnpm-workspace.yaml`; move legacy JSON pnpm overrides to YAML. Sources owns mapped package overrides across the graph and runs pnpm install during dev/release. See the [pnpm setup](https://github.com/refinio/sources#pnpm-local-and-published-packages). Never remove ordinary files/directories to make room for links.

```sh
sources check
sources dev
sources status
sources run lib -- git status --short
```

Ensure `.sources/`, `.sources.local.conf`, and generated workspace links are ignored; commit `.sources.conf`, `.sources.lock`, relevant ignore entries, and changed workspace/package configuration when committing is authorized.

Everything inside `.sources/` and link objects directly inside the configured link directory are managed and replaceable; `sources release --clean` discards managed contents, while original development checkouts must remain intact.

Report changed references, validation results, and any platform link-permission remedy. Opt-in pnpm switching may change workspace overrides and the pnpm lock; commit release-mode files together. Local file packages follow pnpm semantics and can require reinstalling after file changes. Use existing tooling for builds and tests. See the [README](https://github.com/refinio/sources#readme) for release pinning and command details.
