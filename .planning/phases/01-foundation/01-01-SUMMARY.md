---
phase: 01-foundation
plan: 01
subsystem: infra
tags: [rust, cargo, tree-sitter, musl, static-binary]

requires:
  - phase: []
    provides: []

provides:
  - Complete, compiling Rust project with all dependencies pinned and all source modules stubbed
  - Cargo.toml with verified tree-sitter ABI deduplication (no duplicate versions)
  - Release profile optimized for < 15MB musl static binary
  - All 14 source module files declared and importable

affects:
  - All subsequent phases (Plans 02, 03, 04)

tech-stack:
  added:
    - clap 4.6 (CLI argument parsing)
    - gix 0.81 (git context detection)
    - tree-sitter 0.26.8 core + 7 grammar crates
    - serde ecosystem (serde, serde_json, toml, serde_yaml_bw)
    - reqwest 0.13 (HTTP upload)
    - tokio 1.51 (async runtime)
    - rayon 1.11 (parallelism)
    - anyhow 1.0 (error handling)
    - tracing 0.1 + tracing-subscriber 0.3 (logging)
    - ignore 0.4 + globset 0.4 (file discovery)
  patterns:
    - Hard boundary: no tokio imports in src/plugin/ (tokio in upload only)
    - All grammar crates pinned to compatible versions with tree-sitter core
    - Release profile with lto=fat, codegen-units=1 for binary size

key-files:
  created:
    - Cargo.toml (dependencies, release profile)
    - .cargo/config.toml (musl target config)
    - src/main.rs
    - src/types/mod.rs
    - src/vars/mod.rs
    - src/ast/mod.rs
    - src/git/mod.rs
    - src/upload/mod.rs
    - src/core/mod.rs, scanner.rs, resolver.rs, merger.rs, payload.rs
    - src/plugin/mod.rs, config/mod.rs, lang/mod.rs
  modified: []

key-decisions:
  - "Use serde_yaml_bw (not deprecated serde_yaml) for YAML parsing"
  - "Pin all tree-sitter grammar crates to compatible versions; verify no ABI duplicates"
  - "Release profile targets < 15MB binary with lto=fat, codegen-units=1, strip=symbols"
  - "Hard module boundary: tokio only in upload/ and main.rs, never in plugin/"

patterns-established:
  - "Dependency pinning: All crate versions explicitly specified in Cargo.toml"
  - "Binary optimization: Release profile locked in Phase 1 before any logic implemented"
  - "Module stubs: All 14 source files created empty to enable early compilation"

requirements-completed:
  - BLDG-07

duration: 15min
completed: 2026-04-04T13:26:01Z
---

# Phase 01: Foundation Summary

Cargo.toml locked with pinned dependencies and verified tree-sitter ABI deduplication; all 14 source modules created as stubs enabling immediate compilation.

## Performance

- **Duration:** 15 minutes
- **Started:** 2026-04-04T13:21:00Z
- **Completed:** 2026-04-04T13:26:01Z
- **Tasks:** 2 completed
- **Files created:** 16 (Cargo.toml, .cargo/config.toml, 14 source modules)

## Accomplishments

- **Verified tree-sitter ABI compatibility:** All 7 grammar crates (0.23-0.25.x) pinned to tree-sitter 0.26.8 core; `cargo tree --duplicates | grep tree-sitter` produces empty output (no version splits)
- **Binary size configuration locked:** Release profile optimized with `lto=fat`, `codegen-units=1`, `strip=symbols`, `opt-level=z` to target < 15MB musl static binary
- **Full module skeleton created:** All 14 source files exist and compile; hard boundary comment in src/plugin/mod.rs prevents tokio-rayon deadlock (Pitfall 4)
- **Cargo.toml dependency selection:** Replaced deprecated serde_yaml with maintained serde_yaml_bw (v2.5.4); all dependencies at production-grade versions verified on 2026-04-04

## Task Commits

1. **Task 1: Write Cargo.toml with pinned dependencies and release profile** - `effe2f7` (feat)
2. **Task 2: Create all source module stubs for compilation** - `5bb1488` (feat)

## Files Created

- `Cargo.toml` - Package definition, all dependencies pinned, release profile with binary size optimization
- `.cargo/config.toml` - musl target rustflags for static linking
- `src/main.rs` - Minimal entry point (prints "arcanon-scanner")
- `src/core/mod.rs` - Module declarations for scanner, resolver, merger, payload
- `src/core/scanner.rs` - Orchestration pipeline (stub)
- `src/core/resolver.rs` - Intra-repo connection resolver (stub)
- `src/core/merger.rs` - ExtractionResult merger (stub)
- `src/core/payload.rs` - ScanPayloadV1 assembler (stub)
- `src/types/mod.rs` - Shared types module (stub)
- `src/vars/mod.rs` - VariableStore module (stub)
- `src/ast/mod.rs` - tree-sitter wrapper (stub)
- `src/git/mod.rs` - Git context detection (stub)
- `src/upload/mod.rs` - HTTP upload (stub)
- `src/plugin/mod.rs` - Plugin trait declaration with hard tokio boundary
- `src/plugin/config/mod.rs` - Config plugins (stub)
- `src/plugin/lang/mod.rs` - Language plugins (stub)

## Decisions Made

1. **serde_yaml_bw vs serde_yaml:** Used serde_yaml_bw (v2.5.4) because serde_yaml was archived in March 2024 (PITFALLS.md Pitfall 2). serde_yaml_bw is the most actively maintained drop-in replacement with identical API.

2. **tree-sitter version pinning:** All 7 grammar crates pinned to their latest stable versions compatible with tree-sitter 0.26.8 core. Grammar crate ABI is backwards-compatible but not forwards-compatible, so exact pins prevent compile-time version splits (PITFALLS.md Pitfall 1).

3. **Release profile optimization:** Configured `lto=fat` (full LTO), `codegen-units=1` (enables maximum LTO), `strip=symbols` (removes debug), `opt-level=z` (size over speed). These lock in the binary size constraints before any plugin code is written.

4. **Plugin/tokio boundary:** Hard comment in src/plugin/mod.rs stating "NO TOKIO IMPORTS IN THIS DIRECTORY" to prevent Pitfall 4 (rayon/tokio deadlock). Plugins run synchronously on rayon threads; upload runs async on tokio executor.

## Deviations from Plan

**1. [Rule 1 - Bug Fix] Corrected gix and reqwest dependency specifications**
- **Found during:** Task 1 (cargo fetch verification)
- **Issue:** Initial Cargo.toml specified non-existent features for gix and reqwest. Plan text cited features that don't exist in these crate versions.
- **Fix:** Removed feature flags for gix (used default features instead), removed `default-features=false` and `rustls-tls` feature from reqwest (reqwest 0.13 defaults to rustls and the json feature is the only one needed).
- **Files modified:** Cargo.toml
- **Verification:** `cargo fetch` succeeded with zero errors; `cargo tree --duplicates | grep tree-sitter` returned empty (confirmed no version splits).
- **Committed in:** effe2f7 (part of Task 1 commit)
- **Impact:** Necessary for correctness; plan text had outdated feature names. All dependencies resolve correctly and build succeeds with zero errors.

## Issues Encountered

None. The deviation was automatically handled and did not block task completion.

## User Setup Required

None - no external service configuration required for Phase 1. The Rust toolchain (cargo, rustc) must be installed locally, but the plan assumes this is already present.

## Next Phase Readiness

- ✓ Cargo.toml locked with all dependencies pinned and verified
- ✓ cargo build succeeds with zero errors
- ✓ All 14 source modules created and importable
- ✓ Release profile configured for binary size constraints
- ✓ tree-sitter ABI deduplication verified (no duplicate versions)
- ✓ Ready for Plan 02 (implement types, plugin trait, AST wrapper)

No blockers. Phase 01 foundation is complete and stable.

---

*Phase: 01-foundation*
*Plan: 01*
*Completed: 2026-04-04*
