---
phase: 09-resolver-and-tech-debt
plan: 02
subsystem: typescript-plugin, integration-tests
tags: [nestjs, two-phase-extraction, test-hardening, fixture-discovery]
dependency_graph:
  requires: []
  provides: [DEBT-01]
  affects: [integration-tests, typescript-plugin]
tech_stack:
  added: []
  patterns: [tdd, hard-assert, fixture-gitignore-override]
key_files:
  created:
    - tests/fixtures/.gitignore
  modified:
    - src/plugin/lang/typescript.rs
    - tests/integration_test.rs
    - src/core/scanner.rs
decisions:
  - "Added tests/fixtures/.gitignore with !*.json to override root *.json exclusion — prevents walk_repo from skipping package.json in fixture directories"
  - "Kept diagnostic tests (test_nestjs_route_with_import_statement, test_nestjs_extraction_from_fixture, test_walk_repo_finds_ts_files) as permanent regression guards"
metrics:
  duration_minutes: 45
  tasks_completed: 2
  files_modified: 4
  completed_date: "2026-04-06"
---

# Phase 9 Plan 02: NestJS Full Path Assertion Summary

**One-liner:** Hardened NestJS path verification by asserting `/users/:id` in unit and integration tests, and fixed package.json discovery via fixture `.gitignore` override.

## What Was Built

### Task 1: NestJS unit test path assertions

Added three path-level assertions to `src/plugin/lang/typescript.rs`:

- `test_nestjs_route_detection` — added `assert_eq!(endpoint.path, "/users/:id")` after existing method/extraction_method checks
- `test_nestjs_route_no_prefix` (new) — `@Controller('')` + `@Get('/health')` → `/health`
- `test_nestjs_route_post` (new) — `@Controller('/users')` + `@Post('/')` → `/users/`
- `test_nestjs_route_with_import_statement` (new, diagnostic guard) — exact polyglot fixture content with import statement

Also fixed a compile error in `src/core/scanner.rs`: the `make_resolved` test helper was missing the `source_file_hint` field added to `ResolvedLibrary` in a previous plan.

### Task 2: Integration test hard assert + root cause fix

Replaced the soft `eprintln!("WARN: NestJS route not found...")` block with:

```rust
assert!(
    nestjs_endpoint.is_some(),
    "NestJS GET /users/:id not found. ...",
    ...
);
```

**Root cause discovered:** The hard assert revealed that `service-a (endpoints: 0)` in the integration scan. Traced through the full scanner pipeline:

1. Unit test with exact fixture content passed (TypeScript plugin works correctly)
2. `test_nestjs_extraction_from_fixture` — direct plugin call with real fixture files passed  
3. `test_walk_repo_finds_ts_files` — revealed `service-a/package.json` was NOT returned by `walk_repo`

**Root cause:** The project's root `.gitignore` contains `*.json`. The `ignore` crate used by `walk_repo` respects `.gitignore` files, causing all JSON files (including `package.json`) to be excluded from the file walk. Without `package.json`, `detect_frameworks()` never found `@nestjs/core`, so NestJS extraction never ran.

**Fix:** Added `tests/fixtures/.gitignore` with `!*.json` to override the root exclusion within the fixtures directory. This is a targeted override that doesn't affect the rest of the project.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 1 - Bug] Fixed missing `source_file_hint` field in `make_resolved` test helper**
- **Found during:** Task 1 (compile error)
- **Issue:** `src/core/scanner.rs` test helper `make_resolved` created `ResolvedLibrary` without the `source_file_hint` field that was added in plan 09-01
- **Fix:** Added `source_file_hint: String::new()` to the struct literal
- **Files modified:** `src/core/scanner.rs`
- **Commit:** 118cc22

**2. [Rule 1 - Bug] Fixed `package.json` excluded from file walk by root `.gitignore`**
- **Found during:** Task 2 (hard assert revealed NestJS endpoints missing)
- **Issue:** Root `.gitignore` had `*.json` which caused `walk_repo` (using `ignore` crate) to skip all JSON files including `package.json` in test fixtures
- **Fix:** Created `tests/fixtures/.gitignore` with `!*.json` to re-include JSON files within fixture directories
- **Files modified:** `tests/fixtures/.gitignore` (created)
- **Commit:** 885e724

## Commits

| Task | Commit | Message |
|------|--------|---------|
| 1    | 118cc22 | test(09-02): strengthen NestJS unit tests to assert full combined path |
| 2    | 885e724 | fix(09-02): promote NestJS integration assert and fix package.json discovery |

## Self-Check: PASSED

- `src/plugin/lang/typescript.rs` — modified, exists
- `tests/integration_test.rs` — modified, exists
- `tests/fixtures/.gitignore` — created, exists
- Commit 118cc22 — present in git log
- Commit 885e724 — present in git log
- All NestJS tests pass (`cargo test -- test_nestjs`)
- Integration test passes (`cargo test --test integration_test`)
