---
phase: 04
plan: 01
subsystem: Language Plugins and Hardening
tags:
  - AST parsing
  - monorepo scoping
  - plugin scaffolding
dependency_graph:
  requires:
    - 03-05 (full scanner pipeline orchestration)
  provides:
    - AstHelper wrapper for tree-sitter queries
    - service_roots monorepo scoping infrastructure
    - All 7 language plugin stubs for Plans 02-08
  affects:
    - All language plugins (Plans 02-08)
    - Plugin execution flow (scanner.rs)
    - Monorepo service discovery
tech_stack:
  added:
    - tree-sitter streaming iterator pattern
    - HashMap-based ancestor path lookup for scoping
  patterns:
    - "Plugin registry with service_roots passed to all plugins"
    - "Fresh QueryCursor per query execution (memory efficiency)"
    - "Nearest-ancestor scoping via Path::ancestors()"
key_files:
  created:
    - src/ast/mod.rs (AstHelper + QueryMatch)
    - src/plugin/lang/typescript.rs
    - src/plugin/lang/python.rs
    - src/plugin/lang/go.rs
    - src/plugin/lang/java.rs
    - src/plugin/lang/csharp.rs
    - src/plugin/lang/rust_lang.rs
    - src/plugin/lang/ruby.rs
  modified:
    - src/plugin/mod.rs (ExtractionContext + scope_to_service + tests)
    - src/core/scanner.rs (service_roots building and passing)
    - src/plugin/lang/mod.rs (module structure)
    - src/plugin/config/*.rs (all test contexts updated)
decisions:
  - Use streaming_iterator::StreamingIterator for QueryMatches iteration (tree-sitter 0.26.8 API)
  - Build service_roots from config plugins before language plugins run (execution order)
  - Accept HashMap.clone() cost for service_roots (plugins read it, don't modify)
  - Parse tree dropped before query_matches() returns (MEMORY constraint satisfied)
metrics:
  duration: "6 minutes"
  completed_date: "2026-04-04T16:54:20Z"
  tasks_completed: 3
  files_created: 8
  files_modified: 11
  tests_added: 9
  tests_passing: 82/82
---

# Phase 04 Plan 01: Language Plugins and Hardening Summary

**AstHelper wrapper, monorepo scoping, and 7 language plugin stubs for parallel development.**

## Objective Complete

Delivered the foundational infrastructure for all subsequent language plugins: AstHelper for safe query execution, service_roots for monorepo-aware scoping, and full plugin scaffolding.

## What Was Built

### Task 1: AstHelper Wrapper (src/ast/mod.rs)

- **AstHelper struct**: Wraps tree-sitter Parser and QueryCursor
- **query_matches() method**: Executes S-expression queries and returns structured matches
  - Compiles query once per call (production code uses OnceLock for caching)
  - Creates fresh QueryCursor per invocation (Pitfall 6 — never reuse)
  - Drops parse tree before returning (MEMORY constraint)
  - Returns empty Vec on parse or query errors (fault tolerance)
- **QueryMatch struct**: Carrier type with capture_name, node_text (quotes trimmed), and 1-indexed line
- **Tests**: 4 passing
  - String literal extraction with quote trimming (single and double)
  - Invalid query handling (returns empty, no panic)
  - No-match scenario
  - 1-indexed line numbering

### Task 2: ExtractionContext Enhancement and Scoping (src/plugin/mod.rs, src/core/scanner.rs)

- **ExtractionContext field**: `service_roots: HashMap<PathBuf, String>` added
  - Built from config plugin ServiceInfo before language plugins run
  - Empty for single-service repos (backward compatible)
  - Populated by scanner.rs before plugin execution
- **scope_to_service() function**: Nearest-ancestor scoping algorithm
  - Walks file_path.ancestors() from most-specific to least-specific
  - Returns Option<&str> with service name or None if unscoped
  - Pure function, no state, testable in isolation
- **Scanner refactoring**:
  - Extract config plugins and run first (to collect ServiceInfo)
  - Build service_roots HashMap from merged config results
  - Pass service_roots to run_plugins_parallel for all plugins
  - Added is_config_plugin() helper
- **Tests**: 5 passing
  - Exact match and nested file scoping
  - Unscoped files (shared libraries)
  - Nearest ancestor selection (multiple service roots)
  - Empty service_roots map (backward compat)

### Task 3: Language Plugin Stubs (src/plugin/lang/*.rs)

All 7 language plugins scaffolded with identical structure:

| Plugin | File | File Patterns | Plan |
|--------|------|---------------|------|
| TypeScript | typescript.rs | `*.ts, *.tsx, *.js, *.jsx, package.json` | 02 |
| Python | python.rs | `*.py, requirements.txt, pyproject.toml` | 03 |
| Go | go.rs | `*.go, go.mod` | 04 |
| Java | java.rs | `*.java, pom.xml, build.gradle` | 05 |
| C# | csharp.rs | `*.cs, *.csproj` | 06 |
| Rust | rust_lang.rs | `*.rs, Cargo.toml` | 07 |
| Ruby | ruby.rs | `*.rb, Gemfile` | 08 |

- Each returns ExtractionResult::default() (stub)
- Registered in default_plugins() ✓
- All file patterns match architecture spec ✓

## Test Results

**Total**: 82/82 passing
- ast:: 4 tests
- plugin::tests (scope_to_service) 5 tests
- All config plugin tests still passing (70+ tests)

## Deviations from Plan

None. Plan executed exactly as written.

## Key Decisions

1. **streaming_iterator import from tree_sitter**: Rather than add `streaming_iterator` as a dependency, imported the re-export from tree_sitter (already available). Reduces dependency surface.

2. **Service roots built in scanner, not plugins**: Config plugins don't receive service_roots yet (HashMap::new()), only language plugins do. This is correct per MONO-01 — config discovery happens first, then scoping is applied.

3. **HashMap cloning for service_roots**: Each plugin receives a clone of service_roots. Acceptable trade-off for immutability and simplicity (plugins don't modify it, only read). Size negligible (<100 entries typical).

4. **OnceLock pattern documented**: AstHelper::query_matches() recompiles per call for simplicity. Production code should use OnceLock (documented in module-level comment).

## Verification Checklist

- ✓ `cargo build` succeeds with zero errors
- ✓ `cargo test --lib` passes all tests (82/82)
- ✓ All 7 plugin stubs compile and register in default_plugins()
- ✓ AstHelper provides correct text extraction and 1-indexed lines
- ✓ scope_to_service unit tests cover all edge cases
- ✓ ExtractionContext updated everywhere (scanner + all test sites)
- ✓ No tokio imports in src/plugin/ (hard boundary maintained)
- ✓ All file patterns match architecture.md specification

## Known Stubs

All 7 language plugins in src/plugin/lang/ return `ExtractionResult::default()`. These are intentional placeholders implemented in Plans 02-08. No data will flow from these plugins until implementation begins.

## What Comes Next

Plans 02-08 (one per language) now can run in parallel:
- Each plan implements framework detection and AST query extraction
- All plugins will benefit from AstHelper and scope_to_service
- Service scoping will be automatic for all detected connections
- Example: Plan 02 (TypeScript) will use AstHelper to query function calls and HTTP client instantiation

---

**Completed**: 2026-04-04T16:54:20Z  
**Duration**: ~6 minutes  
**Status**: READY FOR PARALLEL LANGUAGE PLUGIN DEVELOPMENT
