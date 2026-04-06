---
phase: 12-wrapper-tracing-refinement
plan: "02"
subsystem: scanner
tags: [wrapper-tracing, deduplication, pattern-engine, connections, hashset]

requires:
  - phase: 12-01-wrapper-tracing-refinement
    provides: WRAPPER_BLOCKLIST extension and docstring skip in detect_wrapper_calls

provides:
  - Post-merge dedup of (source_file_base, protocol) pairs in scanner.rs
  - Pattern-engine connections preferred over wrapper_trace duplicates

affects: [12-03-wrapper-tracing-refinement, testing]

tech-stack:
  added: []
  patterns:
    - "Two-pass HashSet dedup: collect non-wrapper keys first, then filter wrapper connections"
    - "source_file line suffix stripping: split(':').next() normalizes 'path:line' to 'path'"

key-files:
  created: []
  modified:
    - src/core/scanner.rs

key-decisions:
  - "Dedup block placed after wrapper tracing block, before combined_results assembly — minimum scope change"
  - "Inline `use std::collections::HashSet` inside block scope — no new top-level import needed"
  - "source_file line suffix stripped via split(':').next() for robust wrapper/pattern key comparison"

patterns-established:
  - "Wrapper dedup pattern: two-pass HashSet — collect pattern keys, then filter wrapper duplicates"

requirements-completed: [WRAP-11]

duration: 1min
completed: 2026-04-06
---

# Phase 12 Plan 02: Wrapper Tracing Refinement — Dedup Summary

**HashSet-based two-pass dedup in scanner.rs eliminates duplicate connections when both pattern engine and wrapper tracer detect the same (source_file, protocol) pair, keeping the higher-confidence pattern-engine version**

## Performance

- **Duration:** 1 min
- **Started:** 2026-04-06T18:24:11Z
- **Completed:** 2026-04-06T18:25:30Z
- **Tasks:** 1
- **Files modified:** 1

## Accomplishments

- Inserted dedup block between wrapper tracing block and combined_results assembly
- First pass collects all (source_file_base, protocol) keys from non-wrapper (pattern-engine) connections
- Second pass filters wrapper_trace connections that duplicate a pattern-engine key
- source_file line suffix ("path:line" format) stripped to "path" for key comparison
- All 293 existing tests pass after change; cargo build clean

## Task Commits

1. **Task 1: Add pattern+wrapper dedup to scanner.rs (WRAP-11)** - `0fdc52e` (feat)

**Plan metadata:** (to be committed with SUMMARY)

## Files Created/Modified

- `src/core/scanner.rs` — Added 33-line dedup block before combined_results assembly (lines 313-345)

## Decisions Made

- Placed dedup block at the narrowest possible scope (immediately before combined_results assembly) to avoid interfering with the wrapper loop itself
- Used inline `use std::collections::HashSet` — no duplicate top-level import since only `HashMap` was imported at file level
- Two-pass approach (collect then filter) is O(n) in connection count and avoids mutation during the collection pass

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered

None.

## User Setup Required

None - no external service configuration required.

## Next Phase Readiness

- WRAP-11 complete: pattern+wrapper dedup is live
- Ready for 12-03 regression tests covering WRAP-10, WRAP-11, WRAP-12

---
*Phase: 12-wrapper-tracing-refinement*
*Completed: 2026-04-06*
