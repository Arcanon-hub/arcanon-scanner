---
phase: 08-pattern-engine-accuracy
plan: "02"
subsystem: pattern-engine
tags: [testing, regression, dacc-01, dacc-05, py-opcua, py-kubernetes]
dependency_graph:
  requires: []
  provides: [regression-tests-dacc-01, regression-tests-dacc-05]
  affects: [tests/pattern_engine.rs]
tech_stack:
  added: []
  patterns: [helper-function-per-pattern, assert-eq-connection-count]
key_files:
  created: []
  modified:
    - tests/pattern_engine.rs
decisions:
  - "Both DACC-01 and DACC-05 tests added in a single commit since both target tests/pattern_engine.rs"
  - "Helper functions make_opcua_pattern() and make_kubernetes_pattern() keep test setup DRY and mirror the CDN pattern spec exactly"
metrics:
  duration_seconds: 112
  completed_at: "2026-04-06T15:23:53Z"
  tasks_completed: 2
  files_modified: 1
---

# Phase 8 Plan 02: Pattern Engine Regression Tests Summary

Executable regression tests for py-opcua narrowing (DACC-01) and py-kubernetes pattern (DACC-05), locking in correct import_gate and match_str specifications as passing integration tests.

## Tasks Completed

| # | Task | Commit | Files |
|---|------|--------|-------|
| 1 | py-opcua regression tests (DACC-01) | d3d2b83 | tests/pattern_engine.rs (+5 tests) |
| 2 | py-kubernetes pattern tests (DACC-05) | d3d2b83 | tests/pattern_engine.rs (+6 tests) |

## What Was Built

**11 new regression tests** in `tests/pattern_engine.rs`, divided into two sections:

### DACC-01: py-opcua narrowed import_gate and match strings (5 tests)

Helper `make_opcua_pattern()` builds the pattern with the correct narrow spec:
- `import_gate: ["from asyncua import", "from asyncua.", "import asyncua"]`
- `detections[0].match_str: "= Client("`
- `detections[1].match_str: "Client(url="`

Tests verify:
1. `test_opcua_narrowed_import_gate_blocks_substring_match` — bare "asyncua" in a comment does not trigger the gate (old `["asyncua"]` gate would have matched)
2. `test_opcua_narrowed_match_blocks_registry_client` — `RegistryClient("host")` does not match `= Client(` or `Client(url=`
3. `test_opcua_narrowed_match_blocks_governor_signal_client` — `GovernorSignalClient("host")` does not match
4. `test_opcua_assignment_form_fires` — `= Client("opc.tcp://plc:4840")` with asyncua import produces 1 finding with protocol "opcua"
5. `test_opcua_url_kwarg_form_fires` — `asyncua.Client(url="opc.tcp://plc:4840")` produces 1 finding

### DACC-05: py-kubernetes pattern (6 tests)

Helper `make_kubernetes_pattern()` builds the pattern with:
- `import_gate: ["from kubernetes import", "from kubernetes.", "import kubernetes"]`
- 5 detections: `CoreV1Api(`, `AppsV1Api(`, `BatchV1Api(`, `NetworkingV1Api(`, `CustomObjectsApi(`
- All with `protocol: "kubernetes"`, `confidence: High`, `target_extraction: None`

Tests verify:
1. `test_kubernetes_core_v1_api_fires` — CoreV1Api( fires with kubernetes import
2. `test_kubernetes_apps_v1_api_fires` — AppsV1Api( fires with kubernetes import
3. `test_kubernetes_multiple_apis_in_one_file` — 3 distinct API calls in one file produce 3 findings
4. `test_kubernetes_no_import_no_finding` — CoreV1Api( without kubernetes import produces 0 findings
5. `test_kubernetes_custom_objects_api_fires` — CustomObjectsApi( fires
6. `test_kubernetes_networking_v1_api_fires` — NetworkingV1Api( fires with `from kubernetes.client import` form

## Verification Results

```
cargo test "test_opcua" → 5/5 pass
cargo test "test_kubernetes" → 6/6 pass
cargo test (full suite) → 0 FAILED (all test results: ok)
grep -c "fn test_opcua" tests/pattern_engine.rs → 5
grep -c "fn test_kubernetes" tests/pattern_engine.rs → 6
```

## Deviations from Plan

None — plan executed exactly as written. Both task sections were combined into a single commit since the edited file is the same (`tests/pattern_engine.rs`) and both tasks were completed before committing.

## Known Stubs

None — these are pure test additions with no stub data or placeholder values.

## Self-Check: PASSED

- tests/pattern_engine.rs exists and contains 11 new test functions
- commit d3d2b83 exists in git log
- Full suite passes with zero failures
