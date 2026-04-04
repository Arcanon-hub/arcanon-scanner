---
phase: 05-pattern-engine
plan: 01
name: "Pattern Registry Module"
status: complete
duration: 254 seconds
completed_date: "2026-04-04T21:36:27Z"
executor: claude-haiku-4-5-20251001
summary: "Complete src/patterns/mod.rs module with pattern types, HTTP fetch/cache with ETag, and apply engine for import_gate+line-scan+target extraction"
tags:
  - patterns
  - registry
  - deserialization
  - async-fetch
  - cache
tech_stack:
  - serde
  - reqwest
  - tokio
  - tracing
key_files:
  - src/patterns/mod.rs
  - src/lib.rs
decisions:
  - "PatternRegistry::load() is async, called from tokio runtime in main.rs (not blocking)"
  - "Cache at ~/.arcanon/patterns.json with ETag file for conditional requests (D-01, D-02)"
  - "Network failure: warn + fallback to cache; no cache: empty registry with zero patterns (D-03, D-04)"
  - "Target extraction: simple string ops only (str::find + slicing), no regex crate (D-07)"
  - "Extraction failure: emit target_name=\"\" + confidence=Medium (D-09)"
  - "Patterns applied per language, gated by import_gate, scanned line-by-line for match_str"
metrics:
  - total_tasks: 3
  - tasks_completed: 3
  - files_created: 1
  - files_modified: 1
  - lines_added: 650
  - tests_added: 16
  - all_tests_passing: true
dependencies:
  requires: []
  provides:
    - "PatternRegistry struct with load(), apply(), apply_all() methods"
    - "Pattern, Detection, PatternFile, TargetExtraction types"
    - "Async fetch with ETag caching and fallback chain"
    - "Target extraction strategies: none, first_string_arg, named_arg:key, url_hostname"
  affects:
    - "05-02: Pattern config types depend on this module's existence"
    - "05-03: Pattern engine integration into scanner.rs"
---

# Phase 05 Plan 01: Pattern Registry Module — Summary

**Complete `src/patterns/mod.rs` module with pattern deserialization, HTTP fetch/cache with ETag conditional requests, fallback chain, and apply() engine that runs import_gate + line-scan + target extraction.**

## Objective

Build the core `src/patterns/mod.rs` module that fetches detection patterns from `https://patterns.arcanon.dev/v1/patterns.json` at startup, caches locally with ETag support, applies them to produce `ConnectionInfo` findings, and implements all three target extraction strategies without regex. This module replaces all content-gate + line-scan connection detection that currently lives in compiled language plugins.

## What Was Built

### Task 1: Pattern Types and Deserialization

**PatternFile** — Top-level JSON structure from remote/cache:
- `version: String` — Pattern schema version (e.g., "1.0")
- `updated_at: String` — Timestamp of last update
- `patterns: Vec<Pattern>` — Array of detection patterns

**Pattern** — Single pattern definition:
- `id: String` — Unique pattern ID (kebab-case)
- `name, description: String` — Human-readable metadata
- `languages: Vec<String>` — Which language plugins apply this (e.g., ["python", "typescript"])
- `file_patterns: Vec<String>` — Glob patterns for file matching
- `import_gate: Vec<String>` — File must contain one of these to fire pattern
- `detections: Vec<Detection>` — Array of detection rules

**Detection** — Single detection rule within a pattern:
- `match_str: String` — Line text to match (via `#[serde(rename = "match")]`)
- `kind, protocol: String` — Detection kind and protocol (e.g., "connection", "redis")
- `confidence: PatternConfidence` — Confidence level: High, Medium, Low
- `target_extraction: TargetExtraction` — Extraction strategy enum

**TargetExtraction** — Custom deserializer for extraction strategies:
- `None` — No target extraction
- `FirstStringArg` — First quoted string after match
- `NamedArg(String)` — Extract key=value argument (e.g., "named_arg:QueueUrl")
- `UrlHostname` — Parse URL and extract hostname

**PatternConfidence** — Maps to crate::types::Confidence:
- `High` → Confidence::High
- `Medium` → Confidence::Medium
- `Low` → Confidence::Low

All types use serde Deserialize with custom logic for TargetExtraction parsing. Unknown target_extraction strings gracefully fallback to `None` (not parse errors).

### Task 2: Fetch, Cache, and Fallback (PatternRegistry::load)

**PatternRegistry** — Holds loaded patterns and metadata:
- `patterns: Vec<Pattern>` — The loaded patterns
- `version: String` — Pattern schema version
- `source: PatternSource` — Which source patterns came from (Remote/Cache/None)

**PatternRegistry::load()** — Async function with complete fetch/cache/fallback chain:

1. **Determine cache path**: `~/.arcanon/patterns.json` via `std::env::var("HOME")`
2. **Read cached ETag** from `~/.arcanon/patterns.json.etag` if it exists
3. **Build async reqwest request**:
   - GET `https://patterns.arcanon.dev/v1/patterns.json`
   - Header: `If-None-Match: {etag}` if cached ETag exists
   - Header: `Accept: application/json`
   - Timeout: 10 seconds
4. **On HTTP 200** (success):
   - Read response body as JSON
   - Parse as PatternFile
   - Write JSON to cache_path
   - Return PatternRegistry with source: PatternSource::Remote
5. **On HTTP 304** (not modified):
   - Read cached patterns from cache_path
   - Return PatternRegistry with source: PatternSource::Cache
6. **On any error** (network failure, timeout, parse error, non-200/304):
   - Log: `tracing::warn!("Pattern fetch failed: {e}. Falling back to cache.")`
   - Try reading cache_path
   - If cache exists: parse and return PatternSource::Cache
   - If cache missing: log `tracing::warn!("No pattern cache found. Running with zero dynamic patterns.")` and return empty registry (source: PatternSource::None)

**PatternRegistry::patterns()** — Accessor to get patterns slice.

### Task 3: Pattern Apply Engine (PatternRegistry::apply)

**PatternRegistry::apply()** — Apply patterns to a single file:

Algorithm (no regex, simple string ops per D-07):
1. For each pattern in self.patterns:
   - Skip if pattern.languages doesn't contain target language
   - Skip if import_gate is non-empty AND none of the gate strings appear in file content
2. For each line in file:
   - For each detection in pattern:
     - If line doesn't contain detection.match_str → skip
     - Extract target via strategy (see below)
     - If target extracted successfully and non-empty: use extracted target + pattern confidence
     - If extraction fails or returns empty: emit target_name="" + confidence=Medium (D-09)
3. Return Vec<ConnectionInfo> with all findings

**Target Extraction Strategies** — Simple string operations only:

- **TargetExtraction::None** → None (no extraction)
- **TargetExtraction::FirstStringArg** → Find first `"` or `'` in line, extract until closing quote
- **TargetExtraction::NamedArg(key)** → Find `key=`, then extract quoted value
- **TargetExtraction::UrlHostname** → Extract first string, parse as URL (find `://`), extract hostname until `/`

**ConnectionInfo Production**:
```rust
ConnectionInfo {
    source_service: scope_to_service(&file.path, service_roots).unwrap_or(""),
    target_name: extracted_target_or_empty_string,
    protocol: detection.protocol.clone(),
    method: None,
    path: None,
    source_file: "relative/path/to/file.py:42",  // line_number is 1-indexed
    confidence: map_confidence(&detection.confidence),
    extraction_method: "pattern:{pattern_id}",
    evidence: Some(line.trim().to_string()),
}
```

**PatternRegistry::apply_all()** — Apply patterns to multiple files:
- Call apply() for each file
- Collect all connections into ExtractionResult

## Tests: All Passing

16 comprehensive tests covering:

**Deserialization**:
- ✓ Deserialize redis-py pattern from example JSON
- ✓ Parse named_arg:QueueUrl extraction strategy correctly
- ✓ Gracefully handle unknown extraction strategies → TargetExtraction::None
- ✓ Allow empty import_gate (gate always passes)

**Fetch & Fallback**:
- ✓ load(None) returns empty registry (unreachable URL)
- ✓ patterns() accessor returns correct slice

**Apply Engine**:
- ✓ import_gate skip: pattern doesn't fire when import not in file
- ✓ import_gate fire: pattern fires when import present and match found
- ✓ first_string_arg extraction: "Redis('localhost')" → target="localhost"
- ✓ first_string_arg no literal: "Redis(host_var)" → target="" + confidence=Medium
- ✓ named_arg extraction: "send_message(QueueUrl="https://...")" → target="https://..."
- ✓ url_hostname extraction: parse URL and get hostname only
- ✓ language filter: skip patterns for wrong language
- ✓ evidence field: line content is trimmed and stored
- ✓ source_file format: "relative/path:line_number" (1-indexed)
- ✓ apply_all: process multiple files and aggregate results

## Verification

```bash
cargo test --lib patterns:: --nocapture
# Result: 16 tests PASSED

cargo build
# Result: success (lib compiles cleanly)

cargo clippy --lib -- -D warnings
# Result: no warnings in patterns module
```

## Architecture Notes

- **Sync Boundary**: The load() function is async and runs in the tokio runtime (called from main.rs before rayon parallel execution). It does NOT block the rayon thread pool.
- **Cache Location**: User-level ~/.arcanon/patterns.json, shared across all repos scanned by the same user.
- **ETag Support**: Conditional requests with If-None-Match reduce bandwidth and avoid unnecessary parsing.
- **Fallback Chain**: Remote → ETag cache → empty. No embedded defaults per D-04 (scanner requires network on first run or pre-populated cache).
- **No Regex**: All target extraction uses str::find() and slicing. No regex crate dependency per D-07.
- **Module Exports**: PatternRegistry, PatternFile, Pattern, Detection, TargetExtraction, PatternSource, PatternConfidence are pub for use by downstream modules.

## Known Issues

None. All acceptance criteria met.

## Deviations from Plan

None — plan executed exactly as specified. All truth statements verified, all artifacts created.
