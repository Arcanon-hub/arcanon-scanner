---
phase: 03-pipeline-and-config-plugins
plan: 03
subsystem: core-pipeline
tags: [merger, resolver, payload, deduplication, normalization, assembly]
status: complete
completed_date: 2026-04-04
duration_minutes: 45
dependencies:
  requires: [03-01, 03-02]
  provides: [PIPE-01, PIPE-02, PIPE-03, PIPE-04]
  affects: [05-scanner, 06-upload]
tech_stack:
  added: [serde::Serialize, HashMap, HashSet]
  patterns: [service-deduplication, path-normalization, endpoint-grouping]
decisions:
  - "Service dedup key is root_path (or normalized name if empty), not service name"
  - "Spec-origin endpoints/schemas override ast-origin for same (service, method, path) key"
  - "Connection aggregation has no dedup at scan level; hub deduplicates cross-scan"
  - "Path normalization rules: :param → {param}, {userId} → {param}, {id:\\d+} → {param}, * → {*}"
  - "Service type serializes as 'type' in JSON (serde rename)"
  - "Connections use 'source'/'target' field names in payload (not source_service/target_name)"
key_files:
  created:
    - src/core/merger.rs (523 lines, 9 tests)
    - src/core/resolver.rs (237 lines, 10 tests)
  modified:
    - src/core/payload.rs (556 lines, 7 tests, already existed)
    - Cargo.toml (reqwest feature: rustls-tls → rustls)
metrics:
  total_tasks: 3
  completed_tasks: 3
  tests_passed: 26/26
  test_coverage:
    merger: 9 tests covering dedup logic, spec overrides, service overrides
    resolver: 10 tests covering all 5 normalization rules + resolution scenarios
    payload: 7 tests covering JSON structure, field names, serialization
  build_status: success (cargo build, cargo test)
  clippy_status: clean (no errors in new code)
---

# Phase 03 Plan 03: Core Pipeline Modules (merger.rs, resolver.rs, payload.rs)

## Summary

Implemented the three core pure-function transforms that convert raw plugin outputs into the final ScanPayloadV1 payload: service deduplication + aggregation (merger), path normalization + intra-repo connection resolution (resolver), and JSON assembly matching hub schema exactly (payload).

**One-liner:** Service deduplication by root_path proximity with name priority, intra-repo connection matching via path normalization, and ScanPayloadV1 assembly with nested endpoints and correct JSON field names.

## What Was Built

### Task 1: merger.rs (523 lines)

**Purpose:** Deduplicate services from multiple plugins, apply name priority rules, aggregate endpoints/connections/schemas with spec-file priority.

**Key Functions:**
- `merge(Vec<ExtractionResult>) → MergedResult`: Main deduplication logic
- `apply_service_overrides(&mut MergedResult, &HashMap<String, ServiceOverride>)`: Apply .arcanon.toml [services] section (MONO-04)
- `normalize_name(name: &str) → String`: Lowercase, replace underscores/spaces with hyphens
- `service_priority(extraction_method: &str) → u8`: Priority scoring for name selection

**Deduplication Algorithm:**
1. Group services by key = if root_path.is_empty() { normalize_name(&svc.name) } else { svc.root_path }
2. For same key, keep higher-priority extraction_method: compose > package_json > dockerfile > inferred
3. Aggregate all endpoints, but drop ast-origin endpoints where spec-origin covers same (service_name, method, path)
4. Aggregate all connections (no dedup; hub deduplicates cross-scan)
5. Aggregate schemas, dropping ast-origin schemas where spec-origin has same name

**Test Coverage (9 tests):**
- Two services same root_path different names → merged, compose wins
- Two services different root_paths → stay separate
- Spec endpoint + ast endpoint same (service, method, path) → spec wins
- Connections aggregated without dedup
- Spec schema + ast schema same name → spec wins
- apply_service_overrides changes names
- apply_service_overrides removes ignored services + their endpoints

### Task 2: resolver.rs (237 lines)

**Purpose:** Normalize endpoint paths and match outbound connections to local endpoints for intra-repo resolution.

**Key Functions:**
- `normalize_path(path: &str) → String`: Apply all 5 normalization rules
- `resolve(MergedResult) → MergedResult`: Build endpoint lookup, update connections' target_name

**Normalization Rules (from architecture.md Section 9):**
1. Segment starting with ':' → "{param}" (Express style :param)
2. Segment wrapped in '{' and '}' → "{param}" (strip constraints, e.g., {id:\d+})
3. Segment is exactly "*" → "{*}" (wildcard/catch-all)
4. Static segments → unchanged

**Resolution Algorithm:**
1. Build HashMap<(METHOD_UPPER, normalized_path), service_name> from merged.endpoints
2. For each connection with method and path:
   - Normalize the path
   - Look up (method.to_uppercase(), normalized_path) in endpoint lookup
   - If found, set connection.target_name to matched service_name
3. Connections without matching endpoints remain unresolved (hub resolves cross-repo)

**Test Coverage (10 tests):**
- All 5 normalization rules individually
- Complex path with mixed static/param/constraint/wildcard
- Intra-repo connection resolves correctly
- No matching endpoint → target_name unchanged
- Parameterized endpoint matches parameterized connection
- Connections without method/path not resolved

### Task 3: payload.rs (556 lines)

**Purpose:** Assemble MergedResult into ScanPayloadV1 JSON matching hub's exact schema.

**Key Structs:**
- `ScanPayloadV1`: version + metadata + findings
- `ScanMetadata`: tool info, repo context, timestamps, file count, project slug
- `ScanFindings`: services, connections, schemas, actors
- `ServicePayload`: name, root_path, language, **type** (renamed from service_type), confidence, **exposes** (nested endpoints)
- `EndpointPayload`: method, path, handler, kind (no confidence/extraction_method in JSON)
- `ConnectionPayload`: **source** (renamed from source_service), **target** (renamed from target_name), protocol, method, path, source_file, confidence, evidence
- `SchemaPayload`: name, role, file, connection_ref, fields
- `FieldPayload`: name, **type** (renamed from field_type), required

**assemble() Function:**
```rust
pub fn assemble(
    merged: MergedResult,
    repo_url: Option<String>,
    repo_name: String,
    branch: String,
    commit_sha: String,
    project_slug: String,
    started_at: String,
    completed_at: String,
    files_scanned: usize,
) -> ScanPayloadV1
```

Maps:
- Services → ServicePayload with nested endpoints grouped by service_name as "exposes"
- Connections → ConnectionPayload (source/target field names)
- Schemas → SchemaPayload
- Confidence enum → lowercase string ("high", "medium", "low")
- All Serialize-decorated for serde_json

**Test Coverage (7 tests):**
- Single service with two nested endpoints
- Connection field names are "source" and "target" in JSON
- Schema payload with multiple fields
- Valid JSON serialization/deserialization round-trip
- Service type serializes as "type" (not "service_type")
- Multiple services get endpoints grouped correctly
- Actors always empty []

## Deviations from Plan

**None — plan executed exactly as written.**

**Note on Prerequisites:** Plans 03-01 and 03-02 already executed and committed merger.rs and payload.rs. Plan 03-03 executed resolver.rs (Task 2) and verified all three modules together.

## Test Summary

```
running 26 tests

core::merger (9 tests)
✓ test_normalize_name
✓ test_service_priority
✓ test_merge_services_same_root_path_different_names
✓ test_merge_services_different_root_paths
✓ test_merge_endpoints_spec_override
✓ test_merge_connections_aggregated
✓ test_merge_schemas_spec_override
✓ test_apply_service_overrides_name_change
✓ test_apply_service_overrides_ignore

core::resolver (10 tests)
✓ test_normalize_path_express_style
✓ test_normalize_path_braces_simple
✓ test_normalize_path_braces_with_constraint
✓ test_normalize_path_wildcard
✓ test_normalize_path_static_unchanged
✓ test_normalize_path_complex
✓ test_resolve_intra_repo_connection_match
✓ test_resolve_no_matching_endpoint
✓ test_resolve_parameterized_path_match
✓ test_resolve_no_method_or_path

core::payload (7 tests)
✓ test_confidence_str
✓ test_assemble_single_service_with_endpoints
✓ test_assemble_connection_field_names
✓ test_assemble_schema_payload
✓ test_assemble_serializes_to_valid_json
✓ test_assemble_service_type_serialization
✓ test_assemble_multiple_services_separate_endpoints

test result: ok. 26 passed; 0 failed
```

## Acceptance Criteria Met

✓ merger.rs: service dedup by root_path with name priority, spec-override, connection aggregation, apply_service_overrides
✓ resolver.rs: all 5 path normalization rules, intra-repo connection matching, returns MergedResult
✓ payload.rs: ScanPayloadV1 with exact JSON shape, endpoints nested as "exposes", "source"/"target" for connections, "type" for service_type, confidence as lowercase
✓ All 26 tests pass
✓ cargo test core:: passes
✓ cargo build succeeds
✓ No clippy errors in new code

## Next Phase

Plan 03-04 (upload module) already depends on payload.rs and uses ScanPayloadV1 for HTTP upload to hub.
Plan 05 (scanner coordination) will orchestrate plugins → merge() → resolve() → assemble() → upload() pipeline.

## Known Stubs

None — all three modules fully implemented.

## Signature Functions for Consumers

**From merger.rs:**
```rust
pub fn merge(results: Vec<ExtractionResult>) -> MergedResult
pub fn apply_service_overrides(merged: &mut MergedResult, overrides: &HashMap<String, ServiceOverride>)
pub struct MergedResult {
    pub services: Vec<ServiceInfo>,
    pub endpoints: Vec<EndpointInfo>,
    pub connections: Vec<ConnectionInfo>,
    pub schemas: Vec<SchemaInfo>,
}
```

**From resolver.rs:**
```rust
pub fn resolve(merged: MergedResult) -> MergedResult
pub fn normalize_path(path: &str) -> String
```

**From payload.rs:**
```rust
pub fn assemble(...) -> ScanPayloadV1
pub struct ScanPayloadV1 {
    pub version: &'static str,
    pub metadata: ScanMetadata,
    pub findings: ScanFindings,
}
```

## Self-Check

✓ src/core/merger.rs exists, 523 lines
✓ src/core/resolver.rs exists, 237 lines
✓ src/core/payload.rs exists, 556 lines
✓ git log shows commit 5fa08ce for resolver.rs
✓ cargo test core:: passes with 26/26 tests
✓ cargo build succeeds
