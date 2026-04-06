---
phase: 08-pattern-engine-accuracy
plan: "03"
subsystem: pattern-engine
tags: [testing, regression, integration, phase8, opcua, kubernetes]
dependency_graph:
  requires: [08-01, 08-02]
  provides: [TEST-01]
  affects: [tests/pattern_engine.rs]
tech_stack:
  added: []
  patterns: [combined-regression-test, cross-fix-integration]
key_files:
  created: []
  modified:
    - tests/pattern_engine.rs
decisions:
  - "Integration test uses make_opcua_pattern() helper (defined in DACC-01 section) — single source of truth for the narrowed pattern spec"
  - "File_patterns restriction tested via .go and .ts files — verifies globset logic from 08-01 is correctly scoping patterns"
  - "Docstring filtering verified implicitly through combined test — the one real Client( call in non-docstring code is the only finding"
metrics:
  duration_minutes: 40
  completed_date: "2026-04-06"
  tasks_completed: 1
  tasks_total: 1
  files_created: 0
  files_modified: 1
---

# Phase 8 Plan 03: Combined Cross-Fix Integration Tests Summary

**One-liner:** Three combined regression tests confirm DACC-01 (narrowed opcua import gate), DACC-02 (file_patterns glob enforcement), and DACC-04 (Python docstring skip) compose correctly in a single realistic Python file scenario.

## What Was Built

Added a final `TEST-01` section to `tests/pattern_engine.rs` containing three integration tests that verify all Phase 8 fixes interact without conflict:

1. **`test_all_phase8_fixes_combined`** — A realistic Python file with `from asyncua import Client`, a docstring containing `Client("opc.tcp://plc:4840")` as an example, and a real `client = Client(url)` assignment. Verifies exactly one finding fires (the real call), not the docstring example. Exercises DACC-01 (narrowed import gate), DACC-02 (file_patterns), and DACC-04 (docstring skip) together.

2. **`test_phase8_file_patterns_scopes_pattern_to_python_only`** — A `.go` file containing both the asyncua import gate text and `= Client(` match string in comments. Verifies py-opcua's `file_patterns = ["**/*.py"]` correctly excludes `.go` files, producing zero findings. Exercises DACC-02.

3. **`test_phase8_kubernetes_file_patterns_scopes_to_python`** — A `.ts` file containing kubernetes import text and `CoreV1Api()` in comments. Verifies py-kubernetes `file_patterns = ["**/*.py"]` excludes `.ts` files. Exercises DACC-02 for the kubernetes pattern.

## Test Counts

- Before plan 03: 25 tests in `tests/pattern_engine.rs`
- After plan 03: 28 tests in `tests/pattern_engine.rs`
- All 28 tests pass under `cargo test --test pattern_engine`

## Deviations from Plan

None — plan executed exactly as written.

**Note on integration test suite:** `test_polyglot_fixture_end_to_end` in `tests/integration_test.rs` is failing due to pre-existing Phase 9 work-in-progress changes (DEBT-01: NestJS two-phase extraction fix). Those changes to `tests/integration_test.rs` and `src/plugin/lang/typescript.rs` are uncommitted Phase 9 work not part of this plan. The pattern_engine tests, git_test suite, and lib unit tests all pass cleanly.

## Self-Check

Files exist:
- tests/pattern_engine.rs — modified, 3 tests added

Commit exists:
- c6261ac: test(08-03): add combined Phase 8 cross-fix integration tests

## Self-Check: PASSED
