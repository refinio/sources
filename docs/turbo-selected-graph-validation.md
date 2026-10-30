# Selected graph validation, 30 October 2026

The approved correction to PR16 treats `.sources.conf` as an availability allowlist.
Start discovery with the parent native Turbo selection, follow only consumed managed
environment inputs, batch compatible owner/frontier queries, then settle final hashes
bottom up after content preparation and frozen installed-package refresh. Discovery
hashes are provisional. Canonical owner/task identities diagnose cycles and allow
VGER -> AKTE -> a different VGER IMAP task. Phase locks must not span foreign
traversal or native Turbo execution. Selection, configuration and inputs stay stable
throughout the invocation. Native hash-only edges retain their explicit staging
contract. CONTENT edges prepare producers and verify consumed installed bytes.

The public CLI stories cover filtered and empty selections, unused unavailable
allowlisted sources, batched required closures, native prerequisites, child-owned
env and exclusions, return edges, genuine cycles, producer failure, required missing
outputs, and fresh final hashing after native frozen pnpm refresh. Existing content
stories cover deterministic declarations/metadata/assets, changed paths/deletions,
installed corruption, restoration, and development/pinned checkout reuse.

## Disposable real workspace

Evidence is retained at `/home/arne/.cache/sources-selected-graph-owrypnn7`.
Nine repositories were copied with reflinks from the independent VGER recreation.
The package store and Corepack cache are private copies. Absolute managed links and
local mappings were remapped, and 31,151 dependency links were audited with zero
links into the original checkouts. Original repos remain clean. No live consumer,
global tool, profile or release was changed.

Turbo remains 2.8.0. Native owner pnpm versions are preserved, including VGER 9.15.4,
ONE 9.15.0 and AKTE 10.8.1. All nine frozen lockfiles retain their original bytes.
Only disposable Claw and ONE Core build caches were enabled for reuse proof.
Claw consumes 290 declared ONE Core files, including JavaScript, declarations,
`package.json` and the root `@OneObjectInterfaces.d.ts`, with installed counterparts.
The root allowlist retains all 140 tasks. Native refresh uses the declared tooling
root and Claw dependency closure; installed bytes match all 290 source files.

## Measurements

These are single uncontrolled samples in an already installed workspace. Metadata
process lifetimes exclude native producer/build/install phases, but include pnpm
startup. They are not statistically controlled benchmarks or full cold production
qualification. Discovery and final native hash queries are distinguished in retained
traces by provisional empty managed env versus settled values, and first/final query
for the owner without managed inputs.

| Case | Elapsed seconds | pnpm launches | Metadata calls / seconds | Result |
| --- | ---: | ---: | --- | --- |
| Previous PR16 full root task-hash dry query | 122.494 | 156 | not separately measured | baseline |
| Selected graph full root task-hash dry query | 15.773 | 18 | 17 / 10.280 | 49 native tasks |
| Full allowlist Claw initial | 14.143 | 7 | 3 / 2.583 | A MISS, B MISS |
| Claw unchanged | 6.822 | 7 | 3 / 2.310 | A HIT, B HIT |
| Upstream README input edit | 11.890 | 7 | 3 / 2.093 | A MISS, B HIT, all 290 bytes identical |
| Consumed declaration edit | 14.379 | 7 | 3 / 2.330 | A MISS, B MISS |
| Cube full allowlist dry query | 16.788 | 19 | 15 / 9.965 | 26 native tasks, valid IMAP return |
| Cube build | 114.429 | 19 | 15 / 9.855 | success |
| Cube unchanged build | 117.364 | 19 | 15 / 9.506 | success |

For comparison, the historical Sources 0.3.1 full query took 210.827s and native
root Turbo took 1.573s. The query reduction combines selected-graph scoping and
batching; it must not all be attributed to content mode. The old full-allowlist
CONTENT trial failed on caller-lock re-entry; the correction passes unchanged
allowlists and the real AKTE/IMAP return edge.

Claw runs used two provisional discovery calls and one final native hash query,
one upstream Turbo, one native install, one version check and parent Turbo. Full
root task-hash discovery used seven queries and final hashing ten. Cube used six
discovery queries and nine final queries. Cube's parent Turbo took 98.129s initially
and 101.461s unchanged. Twenty-five of its 26 tasks have caching disabled and rebuilt;
the unchanged run is not a no-op acceleration claim.

These real results cover code commit `0cf252c`. Subsequent corrections distinguish
a task literally named `content` from the CONTENT namespace, wait for initial phase
lock contention, and skip discovery for an empty selection. The final binary at
`82b24ef` also passes Claw warm in 7.874s with seven pnpm launches, native refresh
1.699s, A/B HIT, exact installed payloads and unchanged locks. Final repeated Cube
and verification results are retained in the evidence directory.

The original reviewed PR stack and normal correction commits remain intact. Native
Linux checks run locally; macOS/Windows execution remains CI coverage. CI is not
watched. No blanket VGER content migration or production cold-build qualification
is claimed.
