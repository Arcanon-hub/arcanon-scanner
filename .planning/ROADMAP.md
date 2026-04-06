# Roadmap: Arcanon Scanner

## Milestones

- ✅ **v1.0** — Phases 1-7 (shipped 2026-04-05) — [archive](milestones/v1.0-ROADMAP.md)
- 🚧 **v1.1 Detection Accuracy** — Phases 8-10 (in progress)

## Phases

<details>
<summary>✅ v1.0 (Phases 1-7) — SHIPPED 2026-04-05</summary>

- [x] Phase 1: Foundation (4/4 plans) — completed 2026-04-04
- [x] Phase 2: Infrastructure (3/3 plans) — completed 2026-04-04
- [x] Phase 3: Pipeline and Config Plugins (5/5 plans) — completed 2026-04-04
- [x] Phase 4: Language Plugins and Hardening (8/8 plans) — completed 2026-04-04
- [x] Phase 5: Pattern Engine (5/5 plans) — completed 2026-04-05
- [x] Phase 6: Library Resolution (3/3 plans) — completed 2026-04-05
- [x] Phase 7: Wrapper Tracing (3/3 plans) — completed 2026-04-05

</details>

### 🚧 v1.1 Detection Accuracy (In Progress)

**Milestone Goal:** Eliminate false positive explosion in pattern and library detection, fix detection gaps, and close tech debt from v1.0.

- [x] **Phase 8: Pattern Engine Accuracy** - Narrow py-opcua match, enforce file_patterns, filter Python docstrings, add py-kubernetes pattern (completed 2026-04-06)
- [ ] **Phase 9: Resolver and Tech Debt** - Deduplicate library resolution findings, fix NestJS two-phase extraction, implement [services] config parsing
- [ ] **Phase 10: Integration Validation** - End-to-end fixture scan confirms false positive elimination across all fixes

## Phase Details

### Phase 8: Pattern Engine Accuracy
**Goal**: The pattern engine produces no false positives from overly broad matches, respects file scope constraints, and includes the py-kubernetes pattern
**Depends on**: Phase 7
**Requirements**: DACC-01, DACC-02, DACC-04, DACC-05, TEST-01
**Success Criteria** (what must be TRUE):
  1. Scanning an asyncua project emits zero connections for py-opcua unless the source file contains an asyncua import statement
  2. A CDN pattern with `file_patterns = ["*.ts"]` does not match in `.py` or `.go` files
  3. Python source files with triple-quoted docstrings do not produce pattern matches on text inside those docstrings
  4. Scanning a Python project that calls `CoreV1Api(` or `AppsV1Api(` produces a kubernetes connection via the py-kubernetes pattern
  5. Regression tests for all four fixes pass under `cargo test`
**Plans**: 3 plans

Plans:
- [x] 08-01-PLAN.md — Enforce file_patterns glob filter and skip Python docstrings in apply() (DACC-02, DACC-04)
- [x] 08-02-PLAN.md — Regression tests for py-opcua narrowing and py-kubernetes pattern (DACC-01, DACC-05)
- [x] 08-03-PLAN.md — Combined cross-fix integration tests and full suite verification (TEST-01)

### Phase 9: Resolver and Tech Debt
**Goal**: Library resolution emits one finding per library instead of per import line, NestJS endpoint paths are correct in polyglot fixtures, and [services] config overrides are respected
**Depends on**: Phase 7
**Requirements**: DACC-03, DEBT-01, DEBT-02, TEST-02
**Success Criteria** (what must be TRUE):
  1. A Python file with ten `import asyncua` lines produces exactly one library resolution connection for asyncua, not ten
  2. Scanning the polyglot fixture produces NestJS endpoints with full `/users/:id` paths, not bare handler method names
  3. A service with `[services.my-svc] name = "renamed"` in `.arcanon.toml` appears in the payload with the overridden name
  4. A service with `[services.my-svc] ignore = true` in `.arcanon.toml` is absent from the payload
  5. Unit tests for [services] name override, ignore flag, and malformed config all pass under `cargo test`
**Plans**: 3 plans

Plans:
- [x] 09-01-PLAN.md — Fix library resolution amplification (DACC-03)
- [ ] 09-02-PLAN.md — Fix NestJS full path assertion in unit + integration tests (DEBT-01)
- [x] 09-03-PLAN.md — Add [services] config parsing and wire into main.rs (DEBT-02, TEST-02)

### Phase 10: Integration Validation
**Goal**: A full scan of the polyglot fixture repo confirms that all v1.1 fixes collectively produce a false-positive-free result
**Depends on**: Phase 8, Phase 9
**Requirements**: TEST-03
**Success Criteria** (what must be TRUE):
  1. Running `cargo test` on the integration fixture suite passes with the expected connection counts for all services
  2. The fixture scan result contains zero false positive connections that were present before this milestone
  3. The scanner exits 0 and produces a valid ScanPayloadV1 on the polyglot fixture without any regression to v1.0 detections
**Plans**: 1 plan

Plans:
- [ ] 10-01-PLAN.md — Create v1.1 validation fixture and comprehensive integration test (TEST-03)

## Progress

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1. Foundation | v1.0 | 4/4 | Complete | 2026-04-04 |
| 2. Infrastructure | v1.0 | 3/3 | Complete | 2026-04-04 |
| 3. Pipeline and Config Plugins | v1.0 | 5/5 | Complete | 2026-04-04 |
| 4. Language Plugins and Hardening | v1.0 | 8/8 | Complete | 2026-04-04 |
| 5. Pattern Engine | v1.0 | 5/5 | Complete | 2026-04-05 |
| 6. Library Resolution | v1.0 | 3/3 | Complete | 2026-04-05 |
| 7. Wrapper Tracing | v1.0 | 3/3 | Complete | 2026-04-05 |
| 8. Pattern Engine Accuracy | v1.1 | 3/3 | Complete   | 2026-04-06 |
| 9. Resolver and Tech Debt | v1.1 | 2/3 | In Progress|  |
| 10. Integration Validation | v1.1 | 0/1 | Not started | - |
