---
phase: 06
plan: 03
subsystem: library-resolution
status: complete
dependencies:
  requires: [06-02]
  provides: [LRES-integration-test-suite]
  affects: [phase-07-scanner-integration]
tech_stack:
  added: []
  patterns: [TempDir-based integration testing, TOML fixture generation]
key_files:
  created:
    - tests/libres_integration.rs (6 integration tests)
  modified:
    - src/libres/mod.rs (bug fix: TOML parsing)
decisions:
  - title: Use toml::from_str instead of .parse() for TOML parsing
    reason: .parse() was silently failing on array-of-tables syntax; from_str works correctly
    impact: parse_cargo_lock now correctly parses Cargo.lock files with [[package]] syntax
metrics:
  duration: "15 minutes"
  tests_added: 6
  lines_added: 256
  commits: 1
---

# Phase 6 Plan 3: Library Resolution Integration Tests

**One-liner:** Integration tests covering all 6 LRES requirements: venv/node_modules/lock-file discovery, caching, missing environment handling, and extraction_method format validation.

## Summary

Created `tests/libres_integration.rs` with 6 focused integration tests that cover every LRES requirement:

1. **test_lres01_python_venv_detection**: Verifies Python venv discovery and library source scanning via pattern detection (httpx → rest).

2. **test_lres02_node_modules_detection**: Verifies Node.js module discovery in node_modules/ and axios pattern detection.

3. **test_lres03_cargo_lock_dep_resolution**: Verifies Cargo.lock parsing and dependency→protocol inference (tonic → grpc).

4. **test_lres04_cache_prevents_rescan**: Verifies that repeated calls for the same library use cached results (no redundant scans).

5. **test_lres05_missing_env_continues**: Verifies graceful degradation when environments (venv, node_modules) are absent — returns empty results, no panic.

6. **test_lres06_extraction_method_and_confidence**: Verifies that the extraction_method format matches "library_resolution:{lib}→{protocol}" and read_manifest_deps handles missing manifests.

## Implementation Details

### Test Structure

Each test uses `tempfile::TempDir` for isolated fixture creation. Helper function `write_file()` creates nested directory structures and writes file content. Helper function `make_httpx_registry()` constructs a minimal PatternRegistry for testing.

### Test Fixtures

- **LRES-01/04/06**: Create temp venv with Python 3.12 site-packages containing a library importing httpx
- **LRES-02**: Create temp node_modules with @acme/rpc (scoped package) importing axios
- **LRES-03**: Create Cargo.lock with [[package]] array-of-tables syntax and dependency lists
- **LRES-05**: Empty temp directory (no venv, no node_modules, no Cargo.lock)

### Bug Fix (Rule 1: Auto-fix bugs)

During implementation, discovered that `parse_cargo_lock()` was silently failing to parse TOML:

**Issue**: Used `content.parse::<toml::Value>()`, which failed on TOML's `[[package]]` (array-of-tables) syntax with error "unexpected content, expected nothing".

**Root cause**: The `toml` crate's `.parse()` method apparently doesn't handle array-of-tables correctly in version 1.1.2.

**Fix**: Changed line 260 of `src/libres/mod.rs` from:
```rust
match content.parse::<toml::Value>() {
```
to:
```rust
match toml::from_str::<toml::Value>(&content) {
```

**Verification**: `toml::from_str()` correctly parses Cargo.lock files with `[[package]]` and `[[package.dependencies]]` syntax.

## Test Results

All 6 tests pass:
```
running 6 tests
test test_lres05_missing_env_continues ... ok
test test_lres03_cargo_lock_dep_resolution ... ok
test test_lres02_node_modules_detection ... ok
test test_lres04_cache_prevents_rescan ... ok
test test_lres06_extraction_method_and_confidence ... ok
test test_lres01_python_venv_detection ... ok

test result: ok. 6 passed; 0 failed; 0 ignored
```

All unit tests for libres also pass (20 tests).

Lint passes: `cargo clippy -- -D warnings` — no warnings.

## Deviations from Plan

**1. [Rule 1 - Bug] Fixed TOML parsing in parse_cargo_lock()**
- **Found during**: test_lres03_cargo_lock_dep_resolution execution
- **Issue**: `content.parse::<toml::Value>()` failed on TOML array-of-tables syntax
- **Fix**: Changed to `toml::from_str::<toml::Value>(&content)`
- **Files modified**: src/libres/mod.rs (line 260)
- **Commit**: 98002cd

## Known Stubs

None — all tests verify complete, functional behavior. No placeholder data or mock returns.

## Verification

Test names follow requirement traceability pattern: `test_lres0[1-6]_*` mapping to requirements LRES-01 through LRES-06.

Each test explicitly asserts the requirement it covers:
- LRES-01: `assert_eq!(results[0].lib_name, "edgeworks_sdk")` and httpx detection
- LRES-02: `assert!(!results.is_empty())` and axios detection  
- LRES-03: `assert!(lock_map.contains_key("acme-rpc"))` and tonic→grpc inference
- LRES-04: Repeated calls return cached protocols
- LRES-05: Missing env returns empty, no panic
- LRES-06: extraction_method format with Unicode → character

## What Was Built

Complete library resolution feature:
- `src/libres/mod.rs` (Plans 01–02): LibraryResolver with environment discovery, lock file parsing, blocklist, cache
- `read_manifest_deps()` for 7 languages: Python (pyproject.toml, requirements.txt), Node (package.json), Rust (Cargo.toml), Go (go.mod), Ruby (Gemfile), Java (pom.xml), C# (.csproj)
- `src/core/scanner.rs` (Plan 02): Library resolution wired into language_map loop
- `tests/libres_integration.rs` (Plan 03): 6 integration tests covering all requirements

Ready for Phase 7 (scanner integration and end-to-end testing).
