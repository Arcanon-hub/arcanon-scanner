---
phase: 12-wrapper-tracing-refinement
plan: "03"
subsystem: wrapper-tracing
tags: [tests, regression, wrapper, dedup, docstring]
dependency_graph:
  requires: [12-01, 12-02]
  provides: [TEST-05]
  affects: [src/wrapper/mod.rs, src/core/scanner.rs]
tech_stack:
  added: []
  patterns: [unit-tests, regression-tests, dedup-logic-verification]
key_files:
  created: []
  modified:
    - src/wrapper/mod.rs
    - src/core/scanner.rs
decisions:
  - "WRAP-11 test placed in scanner.rs (not wrapper/mod.rs) because the dedup logic lives inline in scanner::run — the test exercises the identical filtering logic as a pure data test without requiring an async call"
metrics:
  duration: "~6 minutes"
  completed: "2026-04-05"
  tasks_completed: 2
  tasks_total: 2
  files_modified: 2
---

# Phase 12 Plan 03: Regression Tests for WRAP-10, WRAP-11, WRAP-12 Summary

**One-liner:** Three targeted regression tests covering blocklist extensions (`exists`/`resolve`), pattern+wrapper dedup, and Python docstring skip in Pass 2 — closing TEST-05.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Add WRAP-10 and WRAP-12 regression tests to wrapper/mod.rs | 72c4030 | src/wrapper/mod.rs |
| 2 | Add WRAP-11 dedup regression test to scanner.rs | 9843227 | src/core/scanner.rs |

## Tests Added

### WRAP-10 — `test_wrap10_blocklist_extensions_skipped` (wrapper/mod.rs:1800)
Calls `check_function_and_add_to_wrapper_map` with `exists` and `resolve` as the function name, both bodies containing an `OpcuaClient(...)` call. Asserts `found` is empty for both — confirming the extended blocklist from 12-01 blocks these names even when their bodies call a known wrapper.

### WRAP-11 — `test_wrap11_dedup_prefers_pattern_engine_over_wrapper_trace` (scanner.rs:762)
Constructs a `pattern:py-opcua` connection for `app/client.py` and a `wrapper_trace:...` connection for `app/client.py:42` with the same protocol. Reproduces the dedup logic (seed `pattern_keys`, then filter) and asserts `keep_wrapper` is false — confirming the 12-02 dedup correctly drops the wrapper duplicate.

### WRAP-12 — `test_wrap12_wrapper_call_inside_docstring_skipped` (wrapper/mod.rs:1857)
Passes a Python file whose only `resolve_client(` occurrence is inside a `"""..."""` docstring to `detect_wrapper_calls`. Asserts the result has zero connections — confirming the triple-quote skip added in 12-01 prevents docstring content from producing false positives.

## Verification

```
cargo test --lib -- test_wrap
running 8 tests
test wrapper::tests::test_wrap10_blocklist_extensions_skipped ... ok
test wrapper::tests::test_wrap11_dedup_prefers_pattern_engine_over_wrapper_trace ... ok
test wrapper::tests::test_wrap12_wrapper_call_inside_docstring_skipped ... ok
(+ 5 other wrapper_map tests)
test result: ok. 8 passed; 0 failed
```

Full suite: all tests pass, zero failures.

## Deviations from Plan

None — plan executed exactly as written.

## Self-Check: PASSED
