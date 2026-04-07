---
phase: 13-payload-schema-and-dedup
plan: "03"
subsystem: dedup
tags: [dedup, connections, scanner, data-quality, dq-03]

# Dependency graph
requires:
  - phase: 13-payload-schema-and-dedup
    provides: ConnectionInfo.dependency field and ConnectionPayload.extraction_method passthrough (13-01, 13-02)
provides:
  - Final dedup pass in scanner.scan() after merger::apply_service_overrides() and before resolver::resolve()
  - extraction_method_score() free function returning u8 priority scores for dedup tie-breaking
affects:
  - ScanPayloadV1 output (fewer duplicate connection rows)
  - DQ-04 and later phases that emit connections (dedup ensures they won't produce duplicate rows)

# Tech tracking
tech-stack:
  added: []
  patterns:
    - "Final dedup after merger, before resolver: (source_file, protocol, target_name) tuple key eliminates duplicate connections from overlapping detectors"
    - "extraction_method_score() priority: pattern=3 > wrapper_trace=2 > library_resolution=1 > others=0"
    - "std::mem::take(&mut merged.connections) pattern avoids borrow conflict while consuming the Vec"

key-files:
  created: []
  modified:
    - src/core/scanner.rs

key-decisions:
  - "Dedup key is (source_file, protocol, target_name) — different target_names are always kept as distinct connections"
  - "Empty string target_name and specific target_name produce different keys — both survive dedup independently"
  - "Step 8.5 comment used despite being positioned after step 10 — preserves semantic labeling from plan spec"

patterns-established:
  - "extraction_method_score(): canonical priority ordering for all future dedup or merge decisions across scanner"

requirements-completed:
  - DQ-03

# Metrics
duration: 5min
completed: 2026-04-07
---

# Phase 13 Plan 03: Final Dedup Pass in scanner.rs Summary

**HashMap-based (source_file, protocol, target_name) dedup pass added to scanner.scan() after merger, before resolver, with extraction_method_score() priority (pattern=3, wrapper_trace=2, library_resolution=1) to eliminate duplicate connections from overlapping detectors**

## Performance

- **Duration:** ~5 min
- **Started:** 2026-04-07T17:10:00Z
- **Completed:** 2026-04-07T17:15:00Z
- **Tasks:** 1
- **Files modified:** 1

## Accomplishments
- Added `extraction_method_score()` free function to scanner.rs (before `#[cfg(test)]` block) returning u8 (3/2/1/0 for pattern/wrapper_trace/library_resolution/others)
- Added Step 8.5 dedup block inside `scan()` after `merger::apply_service_overrides()` and before `resolver::resolve()` using `HashMap<(String, String, String), ConnectionInfo>` with `std::mem::take` for borrow-conflict-free Vec consumption
- Six unit tests covering: score function values, pattern>library_resolution, wrapper_trace>library_resolution, pattern>wrapper_trace, distinct non-empty target_names both kept, empty vs specific target_name coexistence
- All 549 existing tests continue to pass with zero failures

## Task Commits

Each task was committed atomically:

1. **Task 1: Add extraction_method_score() and final dedup block to scanner.rs** - `bbd5e0b` (feat)

## Files Created/Modified
- `src/core/scanner.rs` — Added `extraction_method_score()` free function and Step 8.5 dedup block in `scan()`, plus 6 new unit tests in `#[cfg(test)]` mod

## Decisions Made
- Dedup key includes `target_name` so connections with distinct non-empty targets sharing the same file+protocol are always preserved (plan spec requires this)
- Empty string `target_name` counts as a distinct key value — library_resolution connections (which emit `target_name: ""`) coexist with pattern connections that resolve a specific host, since they carry different information
- `std::mem::take(&mut merged.connections)` used to consume the Vec without a borrow-conflict on `merged` (which is still mutably borrowed below the block)

## Deviations from Plan

### Pre-execution: Worktree sync

The agent worktree (agent-ab78848c) was branched before Plans 13-01 and 13-02 ran. `git merge 891311c` was used to fast-forward the worktree to the wave-1 merge commit that contains the `dependency: Option<String>` field in `ConnectionInfo` and `ConnectionPayload.extraction_method`. This was required for the code to compile and is not a deviation from the plan — it is standard worktree initialization.

No code deviations — plan executed exactly as written after worktree sync.

## Issues Encountered
None after worktree sync.

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- DQ-03 complete: final dedup pass eliminates redundant rows before payload assembly
- Phase 13 complete: all three plans (13-01 DQ-02, 13-02 DQ-01, 13-03 DQ-03) delivered
- Phase 14 (DQ-04: TargetExtraction::EnvDefault) can proceed without dependency on this plan

## Known Stubs
None — dedup pass is fully wired and exercised by 6 unit tests.

---
*Phase: 13-payload-schema-and-dedup*
*Completed: 2026-04-07*
