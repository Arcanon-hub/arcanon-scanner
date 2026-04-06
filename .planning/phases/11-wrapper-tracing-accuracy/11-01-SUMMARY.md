---
phase: 11-wrapper-tracing-accuracy
plan: "01"
subsystem: wrapper
tags: [wrapper-tracing, accuracy, depth-cap, blocklist, false-positives]
dependency_graph:
  requires: []
  provides: [WRAP-08, WRAP-09, TEST-04]
  affects: [wrapper/mod.rs]
tech_stack:
  added: []
  patterns: [module-scope const, early return guard]
key_files:
  created: []
  modified:
    - src/wrapper/mod.rs
decisions:
  - "Depth cap reduced from 5 to 2 — real wrappers are 1-2 hops; depth 5 allowed transitive call graphs to produce 57 wrappers from 6 seeds"
  - "WRAPPER_BLOCKLIST declared at module scope (not inside the function) so detect_wrapper_calls can reference it without duplication"
  - "Fixed-point loop reduced from 0..5 to 0..3 — depth 2 cap converges in at most 2 iterations"
metrics:
  duration: "~4 minutes"
  completed: "2026-04-05"
  tasks_completed: 2
  files_modified: 1
---

# Phase 11 Plan 01: Wrapper Tracing Accuracy — Depth Cap and Blocklist Summary

Reduced wrapper chain depth cap from 5 to 2 and added a 17-name `WRAPPER_BLOCKLIST` at module scope, with early return guards in both Pass 1 (`check_function_and_add_to_wrapper_map`) and Pass 2 (`detect_wrapper_calls`), eliminating the false-positive amplification that produced 231 traced connections in opcua-adapter.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Reduce depth cap to 2 and add WRAPPER_BLOCKLIST | 6d46053 | src/wrapper/mod.rs |
| 2 | Regression tests for depth cap and blocklist | e60b9d2 | src/wrapper/mod.rs |

## Changes Made

### Task 1: Depth Cap and Blocklist

**Edit 1 — Depth guard (`check_function_and_add_to_wrapper_map`):**
- Changed `if new_depth > 5` to `if new_depth > 2`
- Updated log message and comment to reference `(2)` and `WRAP-08`

**Edit 2 — WRAPPER_BLOCKLIST constant (module scope, before `check_function_and_add_to_wrapper_map`):**
- Added 17-name blocklist: `__init__`, `run`, `main`, `start`, `stop`, `setup`, `teardown`, `clear`, `close`, `shutdown`, `cleanup`, `reset`, `init`, `dispose`, `destroy`, `configure`, `register`
- Early return in `check_function_and_add_to_wrapper_map` when `fn_name` is on blocklist

**Edit 3 — Fixed-point loop (`build_wrapper_map`):**
- Changed `for iteration in 0..5` to `for iteration in 0..3`
- Updated doc comment from "max 5 iterations" to "max 3 iterations (depth 2 converges faster)"

**Edit 4 — Blocklist check in `detect_wrapper_calls` Pass 2:**
- Added `if WRAPPER_BLOCKLIST.contains(&wrapper_name.as_str()) { continue; }` after the `depth == 0` guard

### Task 2: Regression Tests (8 tests)

1. `test_check_function_respects_depth_cap` — updated: uses depth 2 in map, not depth 5
2. `test_depth_cap_allows_depth_two` — depth 1 in map → new_depth 2 → IS added
3. `test_depth_cap_blocks_depth_three` — depth 2 in map → new_depth 3 → NOT added
4. `test_blocklist_blocks_init` — `__init__` → NOT added
5. `test_blocklist_blocks_run` — `run` → NOT added
6. `test_blocklist_blocks_main` — `main` → NOT added
7. `test_blocklist_allows_real_wrapper` — `api_get` → IS added
8. `test_detect_wrapper_calls_skips_blocklisted_names` — `__init__` in map at depth 1 → Pass 2 emits no connections

## Verification

```
cargo test --lib wrapper   → 41 passed, 0 failed
cargo clippy -- -D warnings → 0 warnings
```

All 8 required tests present and passing.

Spot-check:
```
grep -n "new_depth > 2\|0\.\.3\|WRAPPER_BLOCKLIST\|__init__" src/wrapper/mod.rs
```
Shows: depth guard at line 656, loop at line 717, WRAPPER_BLOCKLIST declared at line 602 and used at lines 640 and 863.

## Deviations from Plan

### Out-of-scope pre-existing failure

`fastapi_docstring_and_kubernetes` (tests/v1_1_validation.rs) was already failing before this plan's changes (confirmed via `git stash` test). The test expects `asyncua.Client()` in service-fastapi to produce an opcua connection but the CDN pattern is not loaded in the test environment. This failure is not caused by depth cap or blocklist changes and is logged in deferred-items.md.

## Known Stubs

None.

## Self-Check: PASSED

- `src/wrapper/mod.rs` exists and contains all required changes
- Commit `6d46053` exists (Task 1)
- Commit `e60b9d2` exists (Task 2)
- All 8 regression tests pass: `cargo test --lib wrapper` → 41 passed, 0 failed
- `cargo clippy -- -D warnings` → clean
