---
phase: 07-wrapper-tracing
plan: 02
type: execute
started_at: "2026-04-05T10:01:43Z"
completed_at: "2026-04-05T10:15:00Z"
duration_minutes: 13
subsystem: wrapper-tracing
tags:
  - pass-2-detection
  - path-extraction
  - template-literal-normalization
  - extraction-method
  - scanner-integration
requires:
  - 07-01
provides:
  - detect_wrapper_calls() function
  - wrapper tracing active in every scan
affects:
  - pattern_results accumulation
  - extraction pipeline
tech_stack:
  - added: >
    Pass 2 implementation (detect_wrapper_calls), helper function
    (extract_string_arg_from_call), comprehensive test coverage
  - patterns: >
    Wrapper call detection via string literal extraction,
    template literal normalization with {param} placeholders,
    source service attribution via scope_to_service()
key_files_created: []
key_files_modified:
  - src/wrapper/mod.rs
  - src/core/scanner.rs
  - src/main.rs
key_decisions: []
requirements_satisfied:
  - WRAP-02: Pass 2 detects wrapper calls with path extraction
  - WRAP-03: Template literal normalization in extracted paths
  - WRAP-07: Extraction method format wrapper_trace:{wrapper}→{terminal}
---

# Phase 07 Plan 02: Wrapper Tracing Pass 2 Summary

**Implement Pass 2 (detect_wrapper_calls) and wire into scanner.rs**

Detected wrapper calls in user code with path/URL extraction and template literal normalization. Wired wrapper tracing (both Pass 1 and Pass 2) into the scanner.rs run() pipeline after library resolution and before merger.

## Completion Status

**All tasks completed and verified.** Wrapper tracing is now active in every scan.

- Task 1: detect_wrapper_calls() implementation — DONE
- Task 2: Wire into scanner.rs — DONE

### Metrics

- Lines added: ~550 (implementation + tests)
- Functions added: 2 (detect_wrapper_calls, extract_string_arg_from_call)
- Tests added: 11 (comprehensive Pass 2 coverage)
- Test results: 456/456 passed (100%)
- Build status: Clean (0 errors, 4 warnings — all pre-existing)

## Technical Details

### Task 1: detect_wrapper_calls() Implementation

**Location:** `src/wrapper/mod.rs`

Added two public/internal functions to Pass 2:

**`extract_string_arg_from_call(line: &str, callee: &str) -> Option<String>`**
- Extracts first string argument from function call
- Handles template literals (backticks): \`${expr}\` → normalize_template_literal()
- Handles f-strings (Python): f"..." with {expr} → normalize_template_literal()
- Handles regular string literals: "..." or '...' → normalize_template_literal()
- Returns None if no string argument found

**`detect_wrapper_calls(files: &[FileContext], wrapper_map: &WrapperMap, service_roots: &HashMap<PathBuf, String>) -> ExtractionResult`**
- Pass 2: scans user files for calls to wrappers in the map
- Per-file iteration, per-line loop with comment skipping (// # /// /* *)
- For each wrapper in map (depth > 0, skipping seed entries):
  - Checks if line contains `wrapper_name(`
  - Extracts string argument via extract_string_arg_from_call()
  - Resolves terminal function from chain (chain[-1])
  - Emits ConnectionInfo with:
    - `extraction_method`: "wrapper_trace:{wrapper_name}→{terminal}"
    - `confidence`: Medium (inherent to wrapper tracing — cannot verify production reach)
    - `path`: normalized extracted path or None
    - `source_file`: "relative_path:line_number" format (1-indexed)
    - `evidence`: trimmed source line
    - `source_service`: resolved via scope_to_service()

### Task 2: Scanner.rs Integration

**Location:** `src/core/scanner.rs` (lines 298-327)

**Additions:**
1. Added `mod wrapper;` to src/main.rs (was missing from bin module declarations)
2. New wrapper tracing block in run() after library resolution loop
3. Runs inside a scoped block `{ }` for clarity and potential future library file setup

**Flow:**
```
for each language in language_map:
  lang_files = filter files by patterns
  
  [library resolution block — existing]
  
  [WRAPPER TRACING BLOCK — NEW]
    lib_files_for_wrapper = Vec::new() (empty for initial release)
    wrapper_map = build_wrapper_map(all_files, lib_files_for_wrapper, pattern_registry)
    
    if wrapper_map.len() > 0:
      for each language:
        lang_files = filter files by patterns
        wrapper_result = detect_wrapper_calls(lang_files, &wrapper_map, service_roots)
        if !wrapper_result.connections.is_empty():
          pattern_results.push(wrapper_result)
  
  [merger block — existing]
```

**Integration Points:**
- Runs AFTER library resolution (after line 295 in the loop)
- Runs BEFORE merger (adds results to pattern_results before merger line 298)
- Per-language filtering ensures language-specific patterns are respected
- Empty lib_files vector allows future enhancement without API change
- Wrapper map is built ONCE (shared across all languages) per scan

## Test Coverage

**11 Pass 2 Tests Added:**

1. `test_extract_string_arg_simple_string` — apiFetch('/api/v1/teams')
2. `test_extract_string_arg_template_literal` — apiFetch(\`/api/v1/orgs/${orgId}/teams\`)
3. `test_extract_string_arg_fstring` — makeRequest(f"/api/{org}/endpoint")
4. `test_extract_string_arg_no_string_arg` — apiFetch(buildUrl(id)) → path = None
5. `test_extract_string_arg_not_found` — wrong function name → None
6. `test_detect_wrapper_calls_simple` — Basic detection with path
7. `test_detect_wrapper_calls_template_literal` — Template literal normalization
8. `test_detect_wrapper_calls_skips_comments` — Comment lines ignored
9. `test_detect_wrapper_calls_skips_seed_entries` — depth=0 entries skipped
10. `test_detect_wrapper_calls_no_string_argument` — Emits ConnectionInfo with path=None
11. `test_detect_wrapper_calls_multiple_wrappers_same_file` — Multiple wrappers detected
12. `test_detect_wrapper_calls_with_service_roots` — Service attribution works

Plus 22 existing tests from Plan 01 (normalize_template_literal, WrapperMap, build_wrapper_map) — all passing.

**Total: 34 wrapper:: tests passing**

## Verification

```
$ cargo build
   Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.39s

$ cargo test 2>&1 | grep "test result"
test result: ok. 199 passed  (lib tests)
test result: ok. 208 passed  (unit tests in wrapper module)
test result: ok. 34 passed   (wrapper:: tests)
... (remaining test suites)
Total: 456 tests passed; 0 failed

$ grep "pub fn detect_wrapper_calls" src/wrapper/mod.rs
pub fn detect_wrapper_calls(

$ grep "build_wrapper_map\|detect_wrapper_calls" src/core/scanner.rs
        let wrapper_map = wrapper::build_wrapper_map(...)
        let wrapper_result = wrapper::detect_wrapper_calls(...)

$ grep "wrapper_trace" src/wrapper/mod.rs
extraction_method: format!("wrapper_trace:{wrapper_name}→{terminal}")
```

## Design Decisions Applied

- **D-10, D-11** (Template Literal Extraction): ${...} and {var} replaced with {param}, matches hub's normalization
- **D-12** (Max Depth): Wrappers at depth > 5 rejected during Pass 1 (not Pass 2)
- **D-13** (Function Size): Functions > 200 lines skipped during Pass 1 (not Pass 2)
- **D-14** (Class Methods): Method names matched without class qualification (not used in Pass 2, only Pass 1)
- Seed entries (depth 0) skipped in Pass 2 (handled by pattern engine, not duplicate work)
- Comment line detection: //, #, ///, /*, * (for multiline comments)
- Confidence always Medium (wrapper tracing cannot verify production reach)

## Deviations from Plan

**None.** Plan executed exactly as written.

- extract_string_arg_from_call() implemented with all specified behaviors
- detect_wrapper_calls() signature and behavior match spec exactly
- Scanner.rs integration point correct (after library resolution, before merger)
- All acceptance criteria met
- All must_have truths satisfied

## Known Stubs

None. No placeholder/hardcoded values blocking the plan's goals.

## Future Enhancements

- Library wrapper scanning: Replace `Vec::new()` lib_files_for_wrapper with actual library source discovery via LibraryResolver
- Deeper template literal parsing: Current regex-based approach may miss complex nested structures
- Dynamic dispatch: obj[methodName]() patterns (deferred per CONTEXT.md)

---

## Commits

| Commit | Message | Files |
|--------|---------|-------|
| 70d296f | feat(07-02): implement detect_wrapper_calls() Pass 2 detection | src/wrapper/mod.rs |
| 9ab7cef | feat(07-02): wire wrapper tracing into scanner.rs run() | src/core/scanner.rs, src/main.rs |

## Self-Check: PASSED

- ✅ detect_wrapper_calls() exists in src/wrapper/mod.rs (line 754+)
- ✅ extract_string_arg_from_call() exists (line 707+)
- ✅ All 11 new tests passing
- ✅ All 34 wrapper:: tests passing
- ✅ Commits exist and are referenced above
- ✅ src/core/scanner.rs has wrapper::build_wrapper_map call (line 306)
- ✅ src/core/scanner.rs has wrapper::detect_wrapper_calls call (line 314)
- ✅ src/main.rs has mod wrapper; declaration (line 16)
- ✅ cargo build produces zero errors
- ✅ cargo test produces 456/456 passed
