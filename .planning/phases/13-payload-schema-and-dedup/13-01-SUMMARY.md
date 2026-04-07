---
phase: 13-payload-schema-and-dedup
plan: 01
subsystem: types
tags: [connection-info, dependency-tracking, data-model, scan-payload]

# Dependency graph
requires:
  - phase: 12-wrapper-tracing-refinement
    provides: Wrapper tracing with dedup logic used by scanner.rs
provides:
  - ConnectionInfo struct carries dependency: Option<String> field
  - Pattern engine populates dependency from pattern.id
  - Library resolution populates dependency from lib_name
  - Wrapper tracing and compose plugin set dependency to None
affects:
  - 13-02-PLAN.md
  - 13-03-PLAN.md
  - ScanPayloadV1 assembly (payload.rs uses ConnectionInfo)

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "ConnectionInfo.dependency carries detection provenance: Some(id) for pattern/libres, None for wrapper/AST/config"

key-files:
  created: []
  modified:
    - src/types/mod.rs
    - src/patterns/mod.rs
    - src/core/scanner.rs
    - src/wrapper/mod.rs
    - src/plugin/config/compose.rs
    - src/core/merger.rs
    - src/core/resolver.rs
    - src/core/payload.rs

key-decisions:
  - "Wrapper tracing sets dependency: None — seed propagation deferred to a follow-on plan per DQ-02 scope"
  - "Compose plugin sets dependency: None — config-derived connections have no code dependency"
  - "AST/test literals set dependency: None — plugin-emitted connections resolved separately"

patterns-established:
  - "Pattern: dependency field follows detection source — pattern engine sets pattern.id, library resolution sets lib_name, all others None"

requirements-completed:
  - DQ-02

# Metrics
duration: 6min
completed: 2026-04-07
---

# Phase 13 Plan 01: Add dependency Field to ConnectionInfo Summary

**`ConnectionInfo` struct extended with `dependency: Option<String>` populated at all 4 emission sites (pattern engine, library resolution, wrapper tracing, compose plugin) to carry detection provenance through the payload**

## Performance

- **Duration:** 6 min
- **Started:** 2026-04-07T16:59:27Z
- **Completed:** 2026-04-07T17:05:58Z
- **Tasks:** 1
- **Files modified:** 8

## Accomplishments
- Added `pub dependency: Option<String>` after `extraction_method` in `ConnectionInfo` (types/mod.rs)
- Pattern engine sets `dependency: Some(pattern.id.clone())` — every CDN/user pattern match carries its pattern ID
- Library resolution sets `dependency: Some(resolved.lib_name.clone())` — every libres connection carries its library name
- Wrapper tracing and compose plugin set `dependency: None` — no code-level dependency applicable
- All test `ConnectionInfo` literals updated across merger.rs, resolver.rs, payload.rs, scanner.rs
- Added `test_pattern_engine_sets_dependency` and `test_libres_dependency_populated` unit tests
- 543 tests pass with zero failures

## Task Commits

Each task was committed atomically:

1. **Task 1: Add dependency field to ConnectionInfo and fix all construction sites** - `d8e978a` (feat)

## Files Created/Modified
- `src/types/mod.rs` - Added `pub dependency: Option<String>` field to `ConnectionInfo` struct
- `src/patterns/mod.rs` - Set `dependency: Some(pattern.id.clone())` at emission site + new test
- `src/core/scanner.rs` - Set `dependency: Some(resolved.lib_name.clone())` in `build_libres_connections`, updated test helpers, added `test_libres_dependency_populated`
- `src/wrapper/mod.rs` - Set `dependency: None` at wrapper connection emission site
- `src/plugin/config/compose.rs` - Set `dependency: None` at compose connection emission site
- `src/core/merger.rs` - Updated 2 test `ConnectionInfo` literals with `dependency: None`
- `src/core/resolver.rs` - Updated 4 test `ConnectionInfo` literals with `dependency: None`
- `src/core/payload.rs` - Updated 1 test `ConnectionInfo` literal with `dependency: None`

## Decisions Made
- Wrapper tracing sets `dependency: None` — seed propagation from wrapper terminal to originating library is out of scope for this plan (deferred to follow-on task per plan design)
- Compose plugin sets `dependency: None` — `compose-depends_on` connections are service graph edges, not code library dependencies
- AST-derived connections in tests use `dependency: None` — consistent with the rule that AST plugins don't carry a named dependency

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- `ConnectionInfo.dependency` is now populated throughout the pipeline
- Plan 13-02 can use `dependency` for dedup and payload enrichment
- All 543 existing tests pass — no regressions introduced

---
*Phase: 13-payload-schema-and-dedup*
*Completed: 2026-04-07*
