# Requirements: Arcanon Scanner

**Defined:** 2026-04-06
**Core Value:** Accurately detect services, endpoints, and connections across 7 languages and 8 config formats using pure static analysis, producing a complete ScanPayloadV1 for the hub.

## v1.1 Requirements

Requirements for detection accuracy milestone. Each maps to roadmap phases.

### Detection Accuracy

- [x] **DACC-01**: Scanner's `py-opcua` pattern uses narrowed import_gate (`from asyncua import`, `from asyncua.`, `import asyncua`) and match (`= Client(`, `Client(url=`) to eliminate false positives from generic `Client(` substring matching
- [x] **DACC-02**: Pattern engine enforces `file_patterns` field — patterns with file globs only match files whose paths match those globs
- [x] **DACC-03**: Library resolution emits one connection per (library, protocol) pair instead of per-import-line, eliminating false positive amplification
- [x] **DACC-04**: Pattern engine skips Python docstrings (triple-quoted strings) and multi-line string literals when scanning for matches
- [x] **DACC-05**: CDN pattern registry includes `py-kubernetes` pattern detecting `CoreV1Api(`, `AppsV1Api(`, `BatchV1Api(`, `NetworkingV1Api(`, `CustomObjectsApi(` calls

### Tech Debt

- [x] **DEBT-01**: NestJS two-phase extraction produces correct full paths in the polyglot fixture integration test
- [x] **DEBT-02**: Scanner parses `[services]` section from `.arcanon.toml` — supports `name` override and `ignore = true`

### Testing

- [x] **TEST-01**: Each detection accuracy fix (DACC-01 through DACC-05) has regression tests proving the fix works and false positives are eliminated
- [x] **TEST-02**: `[services]` config parsing has unit tests for name override, ignore, and missing/malformed config
- [x] **TEST-03**: End-to-end test scanning a fixture repo validates reduced false positive count after all fixes

## v2 Requirements

### Enhanced Analysis

- **EANA-01**: Variable indirection tracing for config-driven connection targets
- **EANA-02**: External plugin protocol via stdin/stdout JSON
- **EANA-03**: Incremental scanning (only changed files since last commit)

### Distribution

- **DIST-01**: Homebrew tap for macOS installation

## Out of Scope

| Feature | Reason |
|---------|--------|
| LLM enhancement layer | Adds cloud dependency and non-determinism |
| Vulnerability/CVE detection | Snyk/Trivy's domain, not topology |
| Variable indirection tracing | Significant engine change, partial coverage — v2.0 |
| External plugin protocol | v2.0 |
| Incremental scanning | v2.0 |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| DACC-01 | Phase 8 | Complete |
| DACC-02 | Phase 8 | Complete |
| DACC-03 | Phase 9 | Complete |
| DACC-04 | Phase 8 | Complete |
| DACC-05 | Phase 8 | Complete |
| DEBT-01 | Phase 9 | Complete |
| DEBT-02 | Phase 9 | Complete |
| TEST-01 | Phase 8 | Complete |
| TEST-02 | Phase 9 | Complete |
| TEST-03 | Phase 10 | Complete |

**Coverage:**
- v1.1 requirements: 10 total
- Mapped to phases: 10
- Unmapped: 0

---
*Requirements defined: 2026-04-06*
*Last updated: 2026-04-06 after v1.1 roadmap creation*
