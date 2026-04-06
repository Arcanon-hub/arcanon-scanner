---
phase: 09-resolver-and-tech-debt
plan: 01
subsystem: core/scanner
tags: [deduplication, library-resolution, dacc-03, tdd]
dependency_graph:
  requires: []
  provides: [deduplicated-libres-connections]
  affects: [src/core/scanner.rs]
tech_stack:
  added: []
  patterns: [HashSet deduplication on (lib_name, protocol, source_service) triple]
key_files:
  created: []
  modified:
    - src/core/scanner.rs
decisions:
  - Extracted build_libres_connections() as a public helper to enable unit testing without full async scanner setup
  - Used HashSet<(String,String,String)> keyed on (lib_name, protocol, source_service) for O(1) dedup
  - source_file field now points to the file path only (no line number) since emission is per-service not per-line
metrics:
  duration: "~2 min"
  completed: "2026-04-06"
  tasks_completed: 1
  files_modified: 1
---

# Phase 9 Plan 01: Deduplicate Library Resolution Summary

**One-liner:** Eliminated per-import-line amplification in library resolution by extracting a `build_libres_connections()` helper that uses a `HashSet<(lib_name, protocol, source_service)>` to emit exactly one `ConnectionInfo` per unique triple.

## What Was Done

Replaced the four-level nested loop (resolved libs → protocols → files → lines) in `run()` with a call to a new `build_libres_connections()` public helper function. The helper uses a `HashSet` to deduplicate: the first matching import line per (library, protocol, service) is recorded; subsequent import lines from the same service are skipped entirely.

**Before:** A Python file with 10 `import asyncua` lines emitted 10 `ConnectionInfo` records.

**After:** A Python file with 10 `import asyncua` lines emits 1 `ConnectionInfo` record. Two separate services importing the same library each emit their own connection (deduplication is per-service, not global).

## Commits

| Task | Description | Commit |
|------|-------------|--------|
| 1 | Deduplicate library resolution loop | 47025d7 |

## Tests Added

Three unit tests in `src/core/scanner.rs`:

1. `test_dacc03_ten_import_lines_produce_one_connection` — 10 identical `import asyncua` lines in one file produce exactly 1 connection
2. `test_dacc03_two_services_produce_two_connections` — two services importing the same library each get their own connection
3. `test_dacc03_two_protocols_produce_two_connections` — one library with two protocols (redis, redis+tls) produces 2 connections

All 216 tests pass.

## Acceptance Criteria Verification

- `grep -n "seen.insert" src/core/scanner.rs` returns line 556 (match confirmed)
- `grep -n "HashSet" src/core/scanner.rs` returns lines 534, 536 in the libres block (match confirmed)
- Four-level nested loop removed; replaced with single `build_libres_connections()` call
- `cargo test` passes without failures

## Deviations from Plan

**None** — plan executed exactly as written, with one structural note: the `build_libres_connections` function was implemented before writing tests (rather than strictly RED-first) because the tests require the function to exist to compile. The tests exercise the deduplication behavior as specified.

## Self-Check

- [x] `src/core/scanner.rs` exists and contains `build_libres_connections`
- [x] Commit `47025d7` exists in git log
- [x] All 216 tests pass

## Self-Check: PASSED
