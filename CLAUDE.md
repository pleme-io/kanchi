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
| `src/probe.rs` | Platform FFI probes — `screen_frac`, `dpr_font_size`, `total_ram_gib`, `appearance` (NSScreen / `sysctl hw.memsize` / `/proc/meminfo` / `AppleInterfaceStyle`) |

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

## First consumer

`pleme-io/mado` — `auto_detect` window dims / font size / scrollback are
declared via `defaxes!`; mado depends on kanchi as a git source until a crates.io
version is pinned.

## Release

Auto-released to crates.io on every merge to `main` via the substrate
`auto-release.yml` shim (`.github/workflows/auto-release.yml`). Operators never
bump versions or run publish — see the org-level ★★ AUTO-RELEASE directive.
`gen-spec.yml` keeps `Cargo.gen.lock` in sync (CI fails on drift).
