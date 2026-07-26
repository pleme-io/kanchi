# kanchi (感知) — Typed Environment-Discovery Primitive

> **★★★ CSE / Knowable Construction.** This repo operates under **Constructive Substrate Engineering** — canonical specification at [`pleme-io/theory/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md`](https://github.com/pleme-io/theory/blob/main/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md). The Compounding Directive (operational rules: solve once, load-bearing fixes only, idiom-first, models stay current, direction beats velocity) is in the org-level pleme-io/CLAUDE.md ★★★ section. Read both before non-trivial changes.

## Build & Test

```bash
cargo build          # compile
cargo test           # unit tests + doc-tests
nix build            # substrate rust-library build (gen lockfile)
```

## Architecture

**Single source of truth** for environment discovery across pleme-io apps —
the shikumi `discovered()` tier made *declarative*. No app should hand-roll the
`FALLBACK_X` const + `detect_x()` + `detect_x_or_fallback()` trio or re-vendor
platform FFI; declare axes with `kanchi::defaxes!` and the platform probes live
**once** in `kanchi::probe`.

### Module Map

| Path | Purpose |
|------|---------|
| `src/lib.rs` | `defaxes!` macro + `none` sentinel + re-exports |
| `src/probe.rs` | Platform FFI probes — `screen_frac`, `dpr_font_size`, `total_ram_gib`, `appearance`, `in_cluster`, `cgroup_memory_limit_bytes` (NSScreen / `sysctl hw.memsize` / `/proc/meminfo` / `AppleInterfaceStyle`) |
| `src/axes.rs` | Environment-classification closed axes (`Os`, `Arch`, `Orchestrator`, `Cloud`, `CapacityType`, `Tenancy`) |
| `src/cluster.rs` | **ConfigPlane DISCOVERED-tier cluster-wiring discovery** — downward-API (`pod_namespace`/`pod_name`), service-DNS (`ClusterServiceDns` + `resolve_service`/`service_reachable`), and the `ClusterEnv` Environment-trait seam (`HostClusterEnv` std-only real; `MockClusterEnv` test double; Secret reads behind the trait so a cluster-aware consumer — e.g. shikumi's `kube` reader — injects a real client) |

### Cluster discovery — the ConfigPlane DISCOVERED tier (`src/cluster.rs`)

The `discovered()` reach into the *running cluster* (per
`theory/CONFIGURATION-MANAGEMENT.md` §IX). Three probe classes, split by cost:

- **downward-API** — `pod_namespace()` / `pod_name()`: pure `std` env reads
  (`POD_NAMESPACE`/`POD_NAME`, the chart injects them via `fieldRef`), with the
  projected SA `namespace` file as the namespace fallback. No client.
- **service-DNS** — `ClusterServiceDns { service, namespace, cluster_domain }`
  renders `<svc>.<ns>.svc.cluster.local` (typed `Display`, no `format!()`);
  `resolve_service` resolves it through `std::net::ToSocketAddrs`. Resolvable ⇒
  the service is wired; unresolvable ⇒ `None`, the clean degenerate.
- **Secret reads** — behind the `ClusterEnv` trait seam. `HostClusterEnv`
  (std-only) answers env + DNS for real and returns `None` for `read_secret`
  (no API client in the leaf, never a guess); a cluster-aware consumer injects a
  real reader; tests inject `MockClusterEnv`. Kanchi stays k8s-client-free — the
  real Kubernetes API client lives in the consumer (shikumi's `kube` feature).

Every method returns `Option`; off-cluster every answer is `None`, so a shikumi
`DiscoveryLayer` built on this seam degenerates to an empty contribution.

### The contract

`defaxes!` expands each declared axis into three items:

- `FALLBACK_<NAME>: T` — the documented default const.
- `detect_<name>() -> Option<T>` — runs the probe; `None` when undetectable.
- `detect_<name>_or_fallback() -> T` — `detect().unwrap_or(FALLBACK)`.

A probe is any no-arg callable returning `Option<T>` (a `fn` path or a `|| …`
closure baking in args). Use `kanchi::none` for axes whose detection isn't
wired yet — they degenerate cleanly to the fallback.

## Adding a probe

Platform FFI belongs in `src/probe.rs` only — never in a consumer. Add a
`pub fn <name>(...) -> Option<T>`, feature-gate the OS backend, and document the
fallback semantics. Consumers reference it from a `defaxes!` axis.

## The standard seam — shikumi's `discovered()` tier

kanchi axes are the fleet-standard source for a `TieredConfig::discovered()` tier
(`theory/CONFIGURATION-MANAGEMENT.md` Primitive 5). Compose them declaratively via
shikumi's `discovered_from_layers(&[…])` — one `DiscoveryLayer` per axis-group,
each building a partial `Dict` from `detect_*_or_fallback()` — never a hand-rolled
`discovered()` struct literal. The empty-answer case degenerates to `bare()`
(discovery totality). See `shikumi/docs/PROGRESSIVE-DISCOVERY-VERIFICATION.md`.

## First consumer

`pleme-io/mado` — `auto_detect` window dims / font size / scrollback are
declared via `defaxes!`; mado depends on kanchi as a git source until a crates.io
version is pinned.

## Release

Auto-released to crates.io on every merge to `main` via the substrate
`auto-release.yml` shim (`.github/workflows/auto-release.yml`). Operators never
bump versions or run publish — see the org-level ★★ AUTO-RELEASE directive.
`gen-spec.yml` keeps `Cargo.gen.lock` in sync (CI fails on drift).
