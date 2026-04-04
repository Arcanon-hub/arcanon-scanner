---
phase: 01-foundation
plan: 04
subsystem: Build Infrastructure
tags:
  - ci
  - makefile
  - github-actions
  - musl
  - binary-size-assertion
dependency_graph:
  requires:
    - 01-01
    - 01-02
    - 01-03
  provides:
    - Passing Makefile targets (lint, fmt, test, build)
    - GitHub Actions CI pipeline
    - Musl release binary validation
  affects:
    - All future phases depend on CI passing
tech_stack:
  added:
    - Makefile (build automation)
    - GitHub Actions (CI/CD)
  patterns:
    - Tab-based Makefile recipes
    - dtolnay/rust-toolchain for musl target
    - Static cargo caching by Cargo.lock hash
    - Binary size assertion in CI
key_files:
  created:
    - Makefile
    - .github/workflows/ci.yml
  modified:
    - src/ast/mod.rs (added #[allow(dead_code)])
    - src/plugin/mod.rs (added #[allow(dead_code)])
    - src/types/mod.rs (added #[allow(dead_code)])
    - src/vars/mod.rs (added #[allow(dead_code)])
    - src/main.rs (fixed unnecessary closures)
decisions:
  - Phase 1 stubs use #[allow(dead_code)] to suppress clippy warnings — intentional, they will be implemented in later phases
  - Makefile uses tab characters (required by Make) for recipe indentation
  - GitHub Actions uses musl-tools from apt to support x86_64-unknown-linux-musl target on ubuntu-latest
  - Binary size assertion uses wc -c for byte count (not ls -l) to avoid platform differences
  - Cache key based on Cargo.lock hash to ensure reproducibility
metrics:
  duration_minutes: 12
  completed_date: 2026-04-04
  tasks_completed: 2
  commits_created: 2
---

# Phase 01 Plan 04: Build Infrastructure Summary

Write the Makefile and GitHub Actions CI workflow that gate all future work.

## Objective

After this plan, `make lint && make fmt && make test && make build` must all pass locally, and the CI pipeline enforces the same checks on every push. The musl binary size assertion in CI prevents binary bloat from being discovered post-Phase 4.

## Execution Summary

### Task 1: Write Makefile with lint, fmt, test, build targets

**Status:** Complete

Created `Makefile` at repo root with four targets:

```makefile
.PHONY: lint fmt test build

## Run clippy with denied warnings (BLDG-01)
lint:
	cargo clippy -- -D warnings

## Check code formatting (BLDG-02)
fmt:
	cargo fmt --check

## Run all tests (BLDG-03)
test:
	cargo test

## Build debug and release binaries (BLDG-04)
build:
	cargo build
	cargo build --release
```

**Verification:**
- `make lint` — exits 0 (clippy passes)
- `make fmt` — exits 0 (code formatted)
- `make test` — exits 0 (11/11 tests pass)
- `make build` — exits 0 (debug at 5.2MB, release at 670KB — both on macOS)

### Task 2: Write GitHub Actions CI workflow

**Status:** Complete

Created `.github/workflows/ci.yml` with full CI pipeline:

**Steps in order:**
1. Checkout code (`actions/checkout@v4`)
2. Install Rust toolchain with musl target and clippy/rustfmt components (`dtolnay/rust-toolchain@stable`)
3. Install musl linker via apt (`sudo apt-get install -y musl-tools`)
4. Set up cargo cache for registry, git deps, and build artifacts (keyed on `Cargo.lock` hash)
5. Run `cargo clippy -- -D warnings` (BLDG-01)
6. Run `cargo fmt --check` (BLDG-02)
7. Run `cargo test` (BLDG-03)
8. Build musl release binary: `cargo build --release --target x86_64-unknown-linux-musl` (BLDG-06)
9. Check tree-sitter no duplicates: `cargo tree --duplicates | grep tree-sitter` (exits 1 if duplicates, 0 if clean)
10. Assert binary size under 15MB: Uses `wc -c` to get byte count, compares to 15728640 bytes

**Verification:**
- YAML is syntactically valid (parsed with `yaml.safe_load`)
- All required steps present:
  - ✓ `cargo clippy -- -D warnings` (1 match)
  - ✓ `cargo fmt --check` (1 match)
  - ✓ `cargo test` (1 match)
  - ✓ `x86_64-unknown-linux-musl` (3 matches: toolchain target + build command + binary path)
  - ✓ `15728640` bytes limit (1 match)
  - ✓ `tree-sitter` duplicate check (4 matches)
  - ✓ `musl-tools` installation (1 match)
  - ✓ `actions/cache@v4` caching (1 match)

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 2 - Missing Critical Functionality] Added #[allow(dead_code)] to Phase 1 stubs**
- **Found during:** Task 1 (Makefile target execution)
- **Issue:** `cargo clippy -- -D warnings` flagged 15 unused struct and trait method warnings from intentional Phase 1 stubs (AstParser, ExtractionContext, LanguagePlugin trait, ExtractionResult, VariableStore, and type definitions). These stubs will be implemented in Phases 2-4, so the warnings are expected.
- **Fix:** Added `#[allow(dead_code)]` attributes to:
  - `src/ast/mod.rs`: AstParser struct, new(), parse() methods
  - `src/plugin/mod.rs`: FileContext struct, ExtractionContext struct, LanguagePlugin trait
  - `src/types/mod.rs`: Confidence enum, FieldInfo, ActorInfo, ServiceInfo, EndpointInfo, ConnectionInfo, SchemaInfo, ExtractionResult structs
  - `src/vars/mod.rs`: VariableStore struct, new() and resolve() methods
- **Files modified:** src/ast/mod.rs, src/plugin/mod.rs, src/types/mod.rs, src/vars/mod.rs
- **Commits:** 299617f (combined with Makefile creation)

**2. [Rule 1 - Bug Fix] Fixed unnecessary closures in Option handling**
- **Found during:** Task 1 (cargo clippy analysis)
- **Issue:** `main.rs` lines 95-96 used `.or_else(|| ...)` with non-closure expressions, triggering `unnecessary_lazy_evaluations` warning under `-D warnings`
- **Fix:** Changed:
  - `cli.hub_url.or_else(|| file_cfg.scanner.hub_url)` → `cli.hub_url.or(file_cfg.scanner.hub_url)`
  - `cli.project_slug.or_else(|| file_cfg.scanner.project_slug)` → `cli.project_slug.or(file_cfg.scanner.project_slug)`
- **Files modified:** src/main.rs
- **Commits:** 299617f

**Summary:** Both deviations were auto-fixes for clippy lint compliance required to pass `make lint` with `-D warnings`. No plan changes needed; these were necessary for the plan's own verification criteria to pass.

## Requirements Traceability

| Requirement | Status | Evidence |
|-------------|--------|----------|
| BLDG-01 | Complete | `make lint` runs `cargo clippy -- -D warnings` and exits 0 |
| BLDG-02 | Complete | `make fmt` runs `cargo fmt --check` and exits 0 |
| BLDG-03 | Complete | `make test` runs `cargo test` and exits 0 (11/11 tests pass) |
| BLDG-04 | Complete | `make build` produces debug and release binaries, both exit 0 |
| BLDG-05 | Complete | CI workflow triggers on push/PR with lint, fmt, test, musl build steps |
| BLDG-06 | Complete | CI workflow builds `x86_64-unknown-linux-musl` release binary |

## Known Stubs

No stubs that block the plan's goal. The Phase 1 scaffold is complete:
- AST parsing wrapper is stubbed but compiles
- Plugin trait and stubs are defined but not called in main (deferred to Phase 3-4)
- Variable store is stubbed but available for Phase 2 implementation
- All stubs are intentional and documented in code

The main.rs accepts CLI arguments and loads stubs but doesn't run the core scanner yet (intentional for Phase 1 foundation).

## Validation Against Must-Haves

**From plan frontmatter:**

- [x] "`make lint` runs cargo clippy with -D warnings and exits 0 on clean code"
- [x] "`make fmt` runs cargo fmt --check and exits 0 on properly formatted code"
- [x] "`make test` runs cargo test and exits 0 with all tests passing"
- [x] "`make build` builds both debug and release targets and exits 0"
- [x] "GitHub Actions CI workflow file exists and is valid YAML"
- [x] "CI workflow runs lint, fmt, test, and musl release build steps"
- [x] "CI workflow includes tree-sitter duplicate ABI check"
- [x] "CI workflow includes binary size assertion (< 15MB)"

## What's Next

Phase 01 is now gated by automated builds and tests. All subsequent phases depend on:
1. `make lint` passing (no clippy warnings under `-D warnings`)
2. `make fmt` passing (all code formatted)
3. `make test` passing (unit tests covering each module)
4. GitHub Actions CI completing successfully on every push

The next plan (Phase 02) will implement file discovery, git context detection, and variable resolution — all with tests gated by this CI pipeline.

---

*Plan 01-04 executed: 2026-04-04*
*Commits: 299617f, 600dd43*
