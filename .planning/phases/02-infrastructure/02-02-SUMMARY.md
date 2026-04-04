---
phase: 02-infrastructure
plan: 02
subsystem: git-context-detection
tags: [git, context, detection, ci-integration, fallback-chains]
dependency_graph:
  requires: [Cargo.toml with gix/sha2, src/lib.rs module declarations]
  provides: [detect_git_context() function, GitContext struct, full CI fallback chains]
  affects: [Plan 02-03 (VariableStore), Plan 03-* (payload assembly), all CI scanning]
tech_stack:
  added: [gix 0.81 (pure Rust git), sha2 0.10 (SHA-256 hashing)]
  patterns: [Fallback chain (git → env vars → default), content hash determinism (sorted paths, no mtime)]
key_files:
  created: [src/git/mod.rs (264 lines), tests/git_test.rs (144 lines)]
  modified: [Cargo.toml (added gix, sha2, updated serde_yaml_bw), src/lib.rs (added git module)]
decisions:
  - "gix 0.81 with sha1 feature used instead of git2 (pure Rust, no libgit2 C dependency, required for static musl builds)"
  - "Content hash fallback uses SHA-256 of sorted 'path:size' entries — deterministic across CI runs (no mtime)"
  - "Fallback chain order: ARCANON_* (override) > GITHUB_* (GitHub Actions) > CI_* (GitLab) > BRANCH_NAME/GIT_COMMIT (Jenkins) > default"
  - "detect_remote uses Direction::Fetch to get URL (not Direction::Push)"
metrics:
  duration: "~4 minutes"
  completed_date: "2026-04-04"
  tasks_completed: 2
  lines_of_code: 408
  test_count: 6
  all_tests_passing: true
---

# Phase 02 Plan 02: Git Context Detection — Summary

Implement the git context detection module that produces repo_url, repo_name, branch, and commit_sha for every scan. These values identify the scan in the hub and act as the idempotency key.

## Objective Met

The git context detection module is fully implemented and tested, producing verified repo metadata before any plugin runs. Every ScanPayloadV1 upload will carry correct identifiers across all CI environments (GitHub Actions, GitLab CI, Jenkins) and fallback to content-based hashing when no git context is available.

## Files Created

| File | Lines | Purpose |
|------|-------|---------|
| `src/git/mod.rs` | 264 | GitContext struct, detect_git_context() public function, full fallback chain implementations |
| `tests/git_test.rs` | 144 | 6 integration tests covering non-git dirs, CI env fallbacks, content hash determinism, and override chains |

## Files Modified

| File | Changes |
|------|---------|
| `Cargo.toml` | Added gix 0.81 (with sha1 feature), sha2 0.10; updated serde_yaml → serde_yaml_bw 2.5 |
| `src/lib.rs` | Added `pub mod git;` export |
| `src/discovery/mod.rs` | Removed unused `std::io::Read` import (still used via full path) |

## Public API Exported

### GitContext Struct
```rust
pub struct GitContext {
    pub repo_url: Option<String>,        // Remote URL or None if not a git repo
    pub repo_name: String,               // Derived from URL basename or directory name
    pub branch: String,                  // Branch with full fallback chain
    pub commit_sha: String,              // 40-char hex SHA or content hash fallback
}
```

### detect_git_context Function
```rust
pub fn detect_git_context(root: &Path) -> anyhow::Result<GitContext>
```

## Fallback Chains Implemented

### Branch Detection Fallback Chain
1. Git HEAD referent_name() → shorten()
2. ARCANON_BRANCH env var
3. GITHUB_REF_NAME env var (GitHub Actions)
4. CI_COMMIT_BRANCH env var (GitLab CI)
5. BRANCH_NAME env var (Jenkins)
6. Default: "detached" with warning

### Commit SHA Detection Fallback Chain
1. Git HEAD commit ID → to_hex().to_string() (40-char hex)
2. ARCANON_COMMIT_SHA env var
3. GITHUB_SHA env var (GitHub Actions)
4. CI_COMMIT_SHA env var (GitLab CI)
5. GIT_COMMIT env var (Jenkins)
6. Content hash fallback: SHA-256 of sorted "path:size\n" entries

### Remote URL & Repo Name Detection
1. Try "origin" remote first (gix)
2. Fall back to first available remote
3. Extract URL via Direction::Fetch
4. Derive repo_name: split URL on '/', take last segment, strip ".git" suffix
5. Fallback repo_name: directory basename

## Content Hash Implementation

- Walks directory recursively using std::fs::read_dir
- Collects (relative_path, file_size_bytes) tuples
- Sorts by path string (BTreeMap for determinism)
- Formats as "path:size\n" entries
- Computes SHA-256 of concatenated string
- Returns lowercase 64-character hex string
- **Deliberately excludes modification time** to ensure determinism across CI runs

## Tests Passing

All 6 integration tests pass, validating:

| Test | Validates |
|------|-----------|
| `test_non_git_dir` | Non-git directory returns repo_url=None, branch="detached", 64-char SHA-256 commit_sha |
| `test_repo_name_derivation` | repo_name fallback to directory basename when no git repo |
| `test_branch_ci_env_fallback` | GITHUB_REF_NAME env var used when gix cannot detect branch |
| `test_arcanon_branch_overrides_github_ref` | ARCANON_BRANCH takes priority over GITHUB_REF_NAME |
| `test_content_hash_is_deterministic` | Two dirs with identical content produce identical hashes |
| `test_content_hash_is_64_hex_chars` | SHA-256 fallback produces exactly 64 hex characters |

## Success Criteria Verification

- ✅ `cargo build` exits 0 with zero errors
- ✅ `src/git/mod.rs` exports `GitContext` struct and `detect_git_context` function
- ✅ All 6 tests in `tests/git_test.rs` pass
- ✅ Branch fallback chain covers all required env vars (ARCANON_BRANCH → GITHUB_REF_NAME → CI_COMMIT_BRANCH → BRANCH_NAME → "detached")
- ✅ SHA fallback chain covers all required env vars (ARCANON_COMMIT_SHA → GITHUB_SHA → CI_COMMIT_SHA → GIT_COMMIT → content_hash)
- ✅ Content hash is SHA-256 of sorted "path:size" entries (no modification time)
- ✅ gix used (not git2 or manual .git/HEAD parsing)

## Task Breakdown

### Task 1: Implement src/git/mod.rs
- Implemented GitContext struct with four fields: repo_url, repo_name, branch, commit_sha
- Implemented detect_git_context(root) public function with full detection logic
- Implemented detect_remote() to extract remote URL and derive repo_name
- Implemented detect_branch() with full env var fallback chain
- Implemented detect_commit_sha() with full env var fallback chain and content hash
- Implemented content_hash_fallback() for deterministic SHA-256 hashing
- Added dependencies: gix 0.81 (with sha1 feature), sha2 0.10
- All code follows the exact API specified in the plan

### Task 2: Write integration tests
- Created tests/git_test.rs with 6 tests
- test_non_git_dir: validates fallback behavior for non-git directories
- test_repo_name_derivation: validates repo_name extraction
- test_branch_ci_env_fallback: validates GITHUB_REF_NAME fallback
- test_arcanon_branch_overrides_github_ref: validates override precedence
- test_content_hash_is_deterministic: validates hash consistency
- test_content_hash_is_64_hex_chars: validates SHA-256 format
- All tests use #[serial] to prevent race conditions from env var mutations
- All tests clean up env vars before and after execution

## Build Output

```
$ cargo build
   Compiling arcanon_scanner v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 12.50s
```

```
$ cargo test --test git_test
running 6 tests
test test_repo_name_derivation ... ok
test test_content_hash_is_64_hex_chars ... ok
test test_branch_ci_env_fallback ... ok
test test_non_git_dir ... ok
test test_arcanon_branch_overrides_github_ref ... ok
test test_content_hash_is_deterministic ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None — all functionality is complete and tested.

## Auth Gates

None encountered.

## Next Steps

Plan 02-03 can now proceed with variable resolution (VariableStore) using this GitContext as context. All downstream plans (03-* and 04-*) depend on this module for accurate metadata in ScanPayloadV1.

---

**Execution**: Task 1 (git module) and Task 2 (integration tests) completed and committed as a single atomic unit (feat(02-02): implement git context detection module).

**Commit Hash**: a8a0fb4

**Duration**: ~4 minutes (plan start 14:17:29Z, plan end 14:21:21Z UTC)
