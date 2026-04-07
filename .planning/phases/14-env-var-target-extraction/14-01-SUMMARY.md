---
phase: 14-env-var-target-extraction
plan: 01
subsystem: pattern-engine
tags: [env-var, target-extraction, dq-04, backward-scan]
dependency_graph:
  requires: []
  provides: [TargetExtraction::EnvDefault, extract_env_default]
  affects: [src/patterns/mod.rs]
tech_stack:
  added: []
  patterns: [backward-scan-window, ALL_CAPS-env-var-heuristic, tier1-only-languages]
key_files:
  created: []
  modified:
    - src/patterns/mod.rs
decisions:
  - "Backward scan does not key on matched-line var name — scans for any env var pattern in window, extracts default from scan line directly. This avoids false negatives when matched line uses a local variable alias."
  - "Fallback var name resolution order: quoted string from matched line → quoted string from scan window → ALL_CAPS unquoted identifier from matched line → empty string."
  - "Go, C#, and Java (non-@Value) are tier-1 only: emit env:{VAR} immediately without backward scan."
  - "Java @Value is inline-only: extract default from ${VAR:default} pattern on the matched line itself."
  - "extract_unquoted_arg helper added but final fallback uses extract_env_var_ident (ALL_CAPS check) to distinguish env var identifiers from generic local variable names."
metrics:
  duration: "~25 minutes"
  completed: "2026-04-07"
  tasks_completed: 1
  files_changed: 1
---

# Phase 14 Plan 01: EnvDefault Target Extraction Summary

**One-liner:** Backward-scan env var default extractor for Python/TypeScript/Rust/Ruby/Java with 15 unit tests closing DQ-04.

## What Was Built

Added `TargetExtraction::EnvDefault` to the pattern engine (`src/patterns/mod.rs`):

1. **Enum variant** — `EnvDefault` added to `TargetExtraction`, deserializes from `"env_default"` JSON string
2. **`extract_env_default()`** — backward-scans up to 20 lines before the matched line and extracts the default value from language-specific env var patterns
3. **`apply()` integration** — `EnvDefault` branch handled before `extract_target()` call, collects all lines once and passes the window to the extractor
4. **Helper functions** — `extract_second_string_arg`, `extract_after_nullish`, `extract_unwrap_or_arg`, `extract_after_or`, `extract_process_env_var`, `extract_env_var_ident`
5. **15 unit tests** — covering all per-language patterns, boundary conditions, fallback behavior

## Language Support

| Language | Pattern | Extraction |
|----------|---------|-----------|
| Python | `os.getenv("VAR", "default")` | Second quoted arg |
| Python | `os.environ.get("VAR", "default")` | Second quoted arg |
| TypeScript/JS | `process.env.VAR ?? "default"` | After `??` or `\|\|` |
| Rust | `env::var("VAR").unwrap_or("default")` | `.unwrap_or()` arg |
| Ruby | `ENV.fetch("VAR", "default")` | Second quoted arg |
| Ruby | `ENV["VAR"] \|\| "default"` | After `\|\|` |
| Java | `@Value("${VAR:default}")` | Inline `${...}` extraction |
| Go / C# / Java (non-@Value) | tier-1 only | `env:{VAR}` immediately |

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Backward scan strategy revised to not key on matched-line var name**
- **Found during:** Task 1 implementation (GREEN phase iteration)
- **Issue:** The plan's action code used `extract_first_string(matched_line)` as the var name, then searched backward for lines containing that specific name. But matched lines typically contain a local variable alias (e.g. `conn = connect(DATABASE_URL)` → `url` extracted as var name, not `DATABASE_URL`). This caused false negatives for all tests where the matched line used a non-quoted identifier.
- **Fix:** Changed backward scan to pattern-match on any language-specific env var call (not keyed on var name), and extract default directly from the scan line. Added `extract_process_env_var` for TypeScript identifier extraction.
- **Files modified:** `src/patterns/mod.rs`

**2. [Rule 1 - Bug] Fallback var name resolution requires ALL_CAPS heuristic**
- **Found during:** Task 1 implementation (GREEN phase iteration)
- **Issue:** `test_env_default_backward_scan_boundary_exactly_20_lines` expected `env:DATABASE_URL` from `conn = connect(DATABASE_URL)` when no scan window match. Using `extract_unquoted_arg` (any identifier) would have returned `env:some_config_obj` for `test_env_default_unparseable_var_returns_empty` (which expects `""`).
- **Fix:** Added `extract_env_var_ident` that only returns an identifier if it matches `[A-Z][A-Z0-9_]*` (conventional env var naming). `DATABASE_URL` is returned; `some_config_obj` is not.
- **Files modified:** `src/patterns/mod.rs`

**3. [Rule 1 - Bug] `extract_target()` needed `EnvDefault` arm to satisfy exhaustive match**
- **Found during:** First compile attempt
- **Issue:** Rust requires exhaustive matches on enums. `extract_target()` didn't have an arm for `EnvDefault` after the enum was extended.
- **Fix:** Added `TargetExtraction::EnvDefault => None` arm with comment noting it's handled upstream in `apply()`.
- **Files modified:** `src/patterns/mod.rs`

## Self-Check: PASSED

- `src/patterns/mod.rs` — FOUND
- Commit `081478d` — FOUND
- All 15 `test_env_default_*` tests pass
- Full `cargo test` suite: 260 lib tests + 26 integration tests pass (0 failures)
