---
phase: 14-env-var-target-extraction
plan: 02
subsystem: pattern-engine
tags: [env-var, target-extraction, dq-04, integration-tests]
dependency_graph:
  requires: [14-01]
  provides: [DQ-04 integration test coverage for all 10 CDN env-var patterns]
  affects: [tests/pattern_engine.rs, src/patterns/mod.rs]
tech_stack:
  added: []
  patterns: [backward-scan-includes-matched-line, tier1-forward-scan, make_env_pattern-helper]
key_files:
  created: []
  modified:
    - tests/pattern_engine.rs
    - src/patterns/mod.rs
decisions:
  - "Backward scan window extended to lines[scan_start..=line_idx] to include the matched line itself — env var assignment and match_str are often on the same line."
  - "Tier-1 forward scan added: csharp/go/java now scan up to 5 lines forward from matched line to find quoted string when matched line has none (IConfiguration injection pattern)."
  - "C# test uses constructor injection form: IConfiguration on constructor line, GetConnectionString on next line — requires forward scan."
metrics:
  duration: "~20 minutes"
  completed: "2026-04-07"
  tasks_completed: 1
  files_changed: 2
---

# Phase 14 Plan 02: EnvDefault Integration Tests Summary

**One-liner:** 11 DQ-04 integration tests covering all 10 CDN env-var patterns with two bug fixes to extract_env_default enabling same-line and forward-scan extraction.

## What Was Built

Added 11 integration tests to `tests/pattern_engine.rs` and fixed two bugs in `src/patterns/mod.rs` that prevented same-line and forward-scan extraction:

### Tests Added (tests/pattern_engine.rs)

| Test | Pattern | Expected Target |
|------|---------|----------------|
| `test_py_env_getenv_extracts_default` | py-env-getenv | `postgres://localhost/db` |
| `test_py_env_environ_extracts_default` | py-env-environ | `redis://localhost:6379` |
| `test_ts_env_process_extracts_default` | ts-env-process | `postgres://localhost/app` |
| `test_go_env_getenv_emits_hint` | go-env-getenv | `env:DATABASE_URL` |
| `test_rs_env_var_extracts_default` | rs-env-var | `postgres://localhost/dev` |
| `test_rb_env_fetch_extracts_default` | rb-env-fetch | `postgres://localhost/app` |
| `test_rb_env_bracket_extracts_default` | rb-env-bracket | `redis://localhost` |
| `test_java_env_value_annotation_inline` | java-env-value | `jdbc:postgresql://localhost/db` |
| `test_java_env_getenv_emits_hint` | java-env-getenv | `env:DATABASE_URL` |
| `test_cs_env_config_emits_hint` | cs-env-config | `env:DATABASE_URL` |
| `test_env_default_no_default_emits_env_hint` | py-env-getenv (no default) | `env:DATABASE_URL` |

### Helper Added

`make_env_pattern(id, language, match_str, import_gate) -> Pattern` — constructs a Pattern with `TargetExtraction::EnvDefault` for reuse across the 11 tests.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Backward scan excluded the matched line**
- **Found during:** Task 1 GREEN phase
- **Issue:** `extract_env_default` scanned `lines[scan_start..line_idx]` — exclusive of `line_idx`. When `match_str` is `os.getenv(` and the assignment `DATABASE_URL = os.getenv("DATABASE_URL", "default")` is on the matched line, the scan window missed it. All 6 language patterns with same-line env var assignments returned `env:DATABASE_URL` (hint only) instead of the default.
- **Fix:** Changed window to `lines[scan_start..=line_idx]` (inclusive). Since the scan iterates `.rev()`, the matched line is checked first.
- **Files modified:** `src/patterns/mod.rs`
- **Commit:** d7df236

**2. [Rule 1 - Bug] Tier-1 C# forward scan needed for IConfiguration injection pattern**
- **Found during:** Task 1 GREEN phase (test_cs_env_config_emits_hint)
- **Issue:** C# tier-1 code called `extract_first_string(matched_line)` — but the matched line `"public Startup(IConfiguration config) {"` has no quoted string. The env var name `"DATABASE_URL"` appears on the *next* line in `config.GetConnectionString("DATABASE_URL")`.
- **Fix:** For tier-1 languages (go, csharp, java non-@Value), changed extraction to scan `lines[line_idx..line_idx+6]` forward, returning `env:{first_quoted_string_found}`.
- **Files modified:** `src/patterns/mod.rs`
- **Commit:** d7df236

## Self-Check: PASSED

- `tests/pattern_engine.rs` — FOUND
- `src/patterns/mod.rs` — FOUND
- Commit `d7df236` — FOUND
- All 39 pattern_engine integration tests pass (11 new + 28 existing)
- Full `cargo test` suite: 0 failures across all test binaries
- `grep -c "TargetExtraction::EnvDefault" tests/pattern_engine.rs` = 1 (via make_env_pattern helper)
- 11 test function name matches confirmed
