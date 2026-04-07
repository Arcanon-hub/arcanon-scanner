---
gsd_state_version: 1.0
milestone: v1.1
milestone_name: Detection Accuracy
status: verifying
stopped_at: Completed 12-03-PLAN.md
last_updated: "2026-04-07T05:35:44.633Z"
last_activity: 2026-04-07
progress:
  total_phases: 5
  completed_phases: 5
  total_plans: 11
  completed_plans: 11
  percent: 57
---

# Project State

## Project Reference

See: .planning/PROJECT.md (updated 2026-04-06)

**Core value:** Accurately detect services, endpoints, and connections across 7 languages and 8 config formats using pure static analysis, producing a complete ScanPayloadV1 for the hub.
**Current focus:** v1.1 Detection Accuracy — Phase 8 ready to plan

## Current Position

Phase: 8 of 10 (Pattern Engine Accuracy)
Plan: 3 of 3 complete
Status: Phase complete — ready for verification
Last activity: 2026-04-07

Progress: [██████░░░░] 57%

## Accumulated Context

### Decisions

Decisions logged in PROJECT.md Key Decisions table.

Recent decisions affecting current work:

- v1.0 carry-forward: NestJS two-phase extraction broken in polyglot fixture (DEBT-01, Phase 9)
- v1.0 carry-forward: [services] config parsing not implemented (DEBT-02, Phase 9)
- [Phase 09-resolver-and-tech-debt]: ServiceConfig kept separate from merger::ServiceOverride for clean layer separation; main.rs bridges the types
- [Phase 08]: Both DACC-01 and DACC-05 regression tests use helper functions mirroring exact CDN pattern spec
- [Phase 08-pattern-engine-accuracy]: GlobSet compiled per-pattern-per-file (not cached) — acceptable for small pattern counts; TODO comment added for future caching
- [Phase 08-pattern-engine-accuracy]: Triple-quote docstring skip is Python-only — other languages give triple-quotes different semantics
- [Phase 09]: Extracted build_libres_connections() helper to enable unit testing; HashSet dedup on (lib_name, protocol, source_service) triple
- [Phase 08]: Integration test uses make_opcua_pattern() helper — single source of truth for the narrowed pattern spec
- [Phase 09-resolver-and-tech-debt]: Added tests/fixtures/.gitignore with !*.json to fix package.json discovery — root .gitignore had *.json which excluded fixture JSON files from walk_repo
- [Phase 10-integration-validation]: py-kubernetes injected via user_pattern_overrides in tests for CDN-independent self-contained validation
- [Phase 10-integration-validation]: DACC-04 assertions check docstring-originated evidence text, not exact counts, to tolerate libres and wrapper connections
- [Phase 11-wrapper-tracing-accuracy]: Depth cap reduced 5→2 and WRAPPER_BLOCKLIST added: eliminates false-positive amplification while preserving real 1-2 hop wrapper detection
- [Phase 12-wrapper-tracing-refinement]: Extended WRAPPER_BLOCKLIST from 17 to 28 entries adding common Python method names (WRAP-10)
- [Phase 12-wrapper-tracing-refinement]: Used count-based triple-quote toggle for Python docstring skip in detect_wrapper_calls (WRAP-12)
- [Phase 12-02]: Dedup block placed after wrapper tracing block before combined_results assembly; two-pass HashSet approach with source_file line suffix stripping
- [Phase 12-wrapper-tracing-refinement]: WRAP-11 test placed in scanner.rs because dedup logic is inline in scanner::run — pure data test avoids async invocation

### Pending Todos

None.

### Blockers/Concerns

None.

## Session Continuity

Last session: 2026-04-06T18:31:13.121Z
Stopped at: Completed 12-03-PLAN.md
Resume file: None
