# Phase 13: Payload Schema and Dedup - Research

**Researched:** 2026-04-07
**Domain:** Payload serialization, connection deduplication, metadata field population
**Confidence:** HIGH

## Summary

Phase 13 adds two critical data quality improvements to the scanner's output:

1. **extraction_method field**: Every `ConnectionPayload` serialized to the hub must carry metadata about how the connection was detected (pattern engine, wrapper trace, library resolution, AST analysis, or config spec). The internal `ConnectionInfo` struct already computes this across all detection sources; it just needs to be exposed in the serialized JSON payload.

2. **dependency field**: Each connection needs a `dependency` field populated from the source of detection (pattern ID, library name, or seed pattern for wrapper tracing). This enables the hub to filter and correlate connections by their dependency chain.

3. **Final dedup pass**: The scanner currently deduplicates only pattern-engine vs wrapper-trace connections using a (source_file_base, protocol) key at lines 313-354 of scanner.rs. A final dedup pass must occur before payload assembly using a stronger key (source_file, protocol, target_name) with priority ordering: pattern > wrapper > library_resolution.

**Primary recommendation:** 
- Add `extraction_method` and `dependency` fields to `ConnectionPayload` struct in payload.rs (lines 68-77)
- Add `dependency` field to `ConnectionInfo` struct in types/mod.rs (lines 57-67)
- Populate `dependency` in pattern engine (format!("pattern:{}", pattern.id)), wrapper tracing (propagate seed pattern dependency), and library resolution (format!("{}→{}", lib_name, protocol))
- Implement final dedup pass in scanner.rs after merger but before payload assembly, using (source_file, protocol, target_name) as key with priority chain: pattern > wrapper > library_resolution

## Standard Stack

### Core Components Already in Place
| Component | Location | Purpose | Status |
|-----------|----------|---------|--------|
| ConnectionInfo struct | types/mod.rs:57-67 | Internal connection data model | Has `extraction_method`, needs `dependency` |
| ConnectionPayload struct | payload.rs:68-77 | Serialized connection for hub | Missing `extraction_method`, `dependency` |
| Pattern engine | patterns/mod.rs:376-399 | Sets `extraction_method: format!("pattern:{}", pattern.id)` | Implemented; needs `dependency` |
| Wrapper tracing | wrapper/mod.rs:933 | Sets `extraction_method: format!("wrapper_trace:{fn}→{terminal}")` | Implemented; needs `dependency` |
| Library resolution | scanner.rs:608-612 | Sets `extraction_method: format!("library_resolution:{}→{}", lib_name, protocol)` | Implemented; needs `dependency` |
| Parser dedup (WRAP-11) | scanner.rs:313-354 | Deduplicates pattern vs wrapper on (source_file_base, protocol) | Implemented |
| Payload assembly | payload.rs:127-231 | Converts MergedResult to ScanPayloadV1 | Implemented; needs to include new fields |

### No New Dependencies Required
This phase is purely structural — adds fields to existing types and implements dedup logic. No new crates or external dependencies.

## Architecture Patterns

### Current Connection Detection Flow
```
File scan
├── Pattern engine (patterns/mod.rs:apply) → extraction_method: "pattern:{id}"
├── Wrapper tracing (wrapper/mod.rs) → extraction_method: "wrapper_trace:{fn}→{terminal}"
├── Library resolution (scanner.rs, plugin cache) → extraction_method: "library_resolution:{lib}→{proto}"
├── AST plugins (plugin/*.rs) → extraction_method: "ast:{language}"
└── Config plugins (plugin/config/*.rs) → extraction_method: "spec:{type}"

All → ConnectionInfo in ExtractionResult
    ↓
All plugins merged → MergedResult (WRAP-11 dedup only)
    ↓
[PHASE 13 NEW] Final dedup pass → deduplicated MergedResult
    ↓
Payload assembly → ScanPayloadV1 (JSON)
```

### Connection Deduplication Precedence
When multiple detection sources produce the same connection (identified by source_file, protocol, target_name):
1. **Pattern engine** wins (highest confidence, direct detection)
2. **Wrapper trace** wins (inferred from function chains, still fairly direct)
3. **Library resolution** wins (inferred from dependency analysis, lowest confidence)

This is enforced by filtering duplicate extraction methods in order of precedence.

### Dependency Field Semantics
- **Pattern engine**: `dependency: Some("pattern:{pattern_id}")` — which pattern triggered the connection
- **Library resolution**: `dependency: Some("{lib_name}")` — which library dependency is involved
- **Wrapper tracing**: `dependency: propagated_from_seed_pattern` — if the wrapper wraps a pattern-detected connection, inherit its dependency
- **AST plugins**: `dependency: None` — direct code analysis, no dependency intermediary
- **Config plugins**: `dependency: None` — spec-based detection, no code dependency

### Field Addition Points in Codebase

**types/mod.rs** (lines 57-67, ConnectionInfo struct):
```rust
pub struct ConnectionInfo {
    pub source_service: String,
    pub target_name: String,
    pub protocol: String,
    pub method: Option<String>,
    pub path: Option<String>,
    pub source_file: String,
    pub confidence: Confidence,
    pub extraction_method: String,
    pub dependency: Option<String>,  // NEW — "pattern:{id}" | "{lib_name}" | None
    pub evidence: Option<String>,
}
```

**payload.rs** (lines 68-77, ConnectionPayload struct):
```rust
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionPayload {
    pub source: String,
    pub target: String,
    pub protocol: String,
    pub method: Option<String>,
    pub path: Option<String>,
    pub source_file: String,
    pub confidence: String,
    pub extraction_method: String,  // NEW — populated from ConnectionInfo.extraction_method
    pub dependency: Option<String>,  // NEW — populated from ConnectionInfo.dependency
    pub evidence: Option<String>,
}
```

**payload.rs assemble()** (lines 170-184, connection conversion):
```rust
let connections: Vec<ConnectionPayload> = merged
    .connections
    .into_iter()
    .map(|conn| ConnectionPayload {
        source: conn.source_service,
        target: conn.target_name,
        protocol: conn.protocol,
        method: conn.method,
        path: conn.path,
        source_file: conn.source_file,
        confidence: confidence_str(&conn.confidence),
        extraction_method: conn.extraction_method,  // NEW — pass through
        dependency: conn.dependency,  // NEW — pass through
        evidence: conn.evidence,
    })
    .collect();
```

**scanner.rs** (after line 362, new dedup pass):
```rust
// Step 8.5: Final dedup pass — key is (source_file, protocol, target_name)
// Priority: pattern > wrapper > library_resolution
{
    use std::collections::HashMap;
    
    let mut dedup_map: HashMap<(String, String, String), ConnectionInfo> = HashMap::new();
    
    for conn in merged.connections {
        let key = (conn.source_file.clone(), conn.protocol.clone(), conn.target_name.clone());
        
        dedup_map
            .entry(key)
            .and_modify(|existing| {
                // Determine if new connection should replace existing based on priority
                let new_priority = extraction_method_priority(&conn.extraction_method);
                let existing_priority = extraction_method_priority(&existing.extraction_method);
                
                if new_priority > existing_priority {
                    *existing = conn.clone();
                }
            })
            .or_insert(conn);
    }
    
    merged.connections = dedup_map.into_values().collect();
}

fn extraction_method_priority(method: &str) -> u8 {
    if method.starts_with("pattern:") {
        3
    } else if method.starts_with("wrapper_trace:") {
        2
    } else if method.starts_with("library_resolution:") {
        1
    } else {
        0  // ast_*, spec:*, etc.
    }
}
```

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Deduplication by custom key | Custom HashMap with multi-field comparison | HashMap<(String, String, String), ConnectionInfo> with tuple keys | Rust's tuple equality and hashing is optimized; avoid manual concatenation |
| Field serialization differences | Custom serialization logic per detection source | serde #[serde(rename)] + struct field naming | Keeps JSON field names separate from Rust field names; already used for other renames |
| Filtering by extraction method pattern | String contains() checks | Prefix matching (starts_with) | More efficient, unambiguous extraction method parsing |

## Common Pitfalls

### Pitfall 1: Forgetting to Populate dependency in All Detection Sources
**What goes wrong:** Some detection paths (e.g., AST plugins, config plugins) populate ConnectionInfo but leave `dependency: None` unset, causing Option handling issues downstream.

**Why it happens:** dependency is a new field; existing code paths that create ConnectionInfo won't know about it until explicitly updated. Rust's compiler will force additions where the struct is instantiated, but forgetting in one place means uninitialized state.

**How to avoid:** 
1. After adding `dependency: Option<String>` to ConnectionInfo, Rust will immediately flag all `ConnectionInfo { ... }` instantiations that don't include it.
2. Add tests for each detection source (pattern, wrapper, libres, ast, spec) verifying dependency is populated correctly.
3. In pattern engine (patterns/mod.rs:388-401), set `dependency: Some(format!("pattern:{}", pattern.id))` or `Some(pattern.id.clone())`.
4. In wrapper tracing (wrapper/mod.rs:933), propagate dependency from seed pattern if available, or None if wrapper itself doesn't have it.
5. In library resolution (scanner.rs:612), set `dependency: Some(resolved.lib_name.clone())`.
6. In AST plugins, set `dependency: None`.
7. In config plugins, set `dependency: None`.

**Warning signs:** 
- Compilation errors on ConnectionInfo instantiation
- Runtime panics or Option unwrap failures in payload assembly
- Payload JSON with missing `dependency` field (serde will serialize Option::None as null)

### Pitfall 2: Dedup Key Collision with Empty Targets
**What goes wrong:** Library resolution often produces connections with empty target_name (e.g., `target_name: ""`). If two different detection sources produce the same (source_file, protocol, "") pair, the second one incorrectly deduplicates against the first even though they should both exist.

**Why it happens:** Library resolution cannot extract specific hostnames from dependency analysis — it only knows "this file uses the redis crate", not "this file connects to redis://cache.example.com". So dedup by (source_file, protocol) is too broad when target_name is empty.

**How to avoid:**
- Dedup key includes target_name as the third element: `(source_file, protocol, target_name)`
- When target_name is empty, the key becomes `(source_file, protocol, "")`, which is a distinct entry
- Same (source_file, protocol, "http://api.example.com") and (source_file, protocol, "") coexist
- This allows library resolution's generic "this uses HTTP" to coexist with pattern engine's specific "this calls api.example.com"

**Warning signs:**
- Scanning a file with both pattern-engine connections and library-resolution connections produces fewer connections in payload than expected
- Manual inspection shows library-resolution connections missing after dedup

### Pitfall 3: Dedup Happening at Wrong Stage
**What goes wrong:** Dedup is applied at scanner.rs:313-354 (pattern vs wrapper only), but that's too early. Library resolution connections are created later (line 586-620), so the final dedup doesn't see them.

**Why it happens:** The existing dedup pass only compares pattern and wrapper results because those are combined before the step. Library resolution is computed separately and added to `all_results` at line 358. By the time merger runs, library resolution connections are already in the merged result.

**How to avoid:**
- Implement final dedup AFTER merger.merge() at line 362 but BEFORE resolver.resolve() at line 372
- At this point, merged.connections contains all connections from all sources (pattern, wrapper, libres, AST, config)
- The dedup pass then has complete visibility into all potential duplicates

**Warning signs:**
- Final payload contains connections from multiple sources with identical source_file, protocol, target
- Dedup logic at scanner.rs:313-354 never removes library_resolution entries (because they don't exist yet)

### Pitfall 4: extraction_method Not Serialized to JSON
**What goes wrong:** ConnectionPayload has the extraction_method field but serde doesn't serialize it, so the JSON payload reaches the hub without the metadata the planner intended to add.

**Why it happens:** payload.rs lines 170-184 convert ConnectionInfo to ConnectionPayload but the mapping doesn't include the new fields. Serde only serializes fields that exist in the struct and are referenced in the mapping.

**How to avoid:**
1. Add `extraction_method: String` and `dependency: Option<String>` to ConnectionPayload struct.
2. In the assemble() function's connection mapping, explicitly include both:
   ```rust
   extraction_method: conn.extraction_method,
   dependency: conn.dependency,
   ```
3. Write a test that serializes a ConnectionPayload and verifies the JSON contains both fields.

**Warning signs:**
- `cargo test` passes but manual JSON inspection shows missing fields
- Hub integration tests fail because expected fields are null in the payload

## Code Examples

### Pattern Engine extraction_method and dependency
**Source:** patterns/mod.rs, lines 388-401

```rust
findings.push(ConnectionInfo {
    source_service: crate::plugin::scope_to_service(&file.path, service_roots)
        .unwrap_or("")
        .to_string(),
    target_name,
    protocol: detection.protocol.clone(),
    method: None,
    path: None,
    source_file: format!("{}:{}", file.relative_path, line_number + 1),
    confidence,
    extraction_method: format!("pattern:{}", pattern.id),
    dependency: Some(pattern.id.clone()),  // NEW
    evidence: Some(line.trim().to_string()),
});
```

### Wrapper Tracing extraction_method and dependency
**Source:** wrapper/mod.rs, line 933 and surrounding context

```rust
// When emitting a wrapper-traced connection, include the chain
result.connections.push(ConnectionInfo {
    source_service: scope.clone(),
    target_name: extract_path(...).unwrap_or_default(),
    protocol: wrapper.protocol.clone(),
    method: None,
    path: path_extracted,
    source_file: format!("{}:{}", file.relative_path, line_num + 1),
    confidence: Confidence::High,
    extraction_method: format!("wrapper_trace:{wrapper_name}→{terminal}"),
    dependency: seed_pattern_dependency.clone(),  // NEW — propagate from seed
    evidence: Some(line.trim().to_string()),
});
```

### Library Resolution extraction_method and dependency
**Source:** scanner.rs, lines 600-613

```rust
libres_connections.push(crate::types::ConnectionInfo {
    source_service,
    target_name: String::new(),
    protocol: protocol.clone(),
    method: None,
    path: None,
    source_file: file.relative_path.clone(),
    confidence: crate::types::Confidence::Medium,
    extraction_method: format!(
        "library_resolution:{}→{}",
        resolved.lib_name, protocol
    ),
    dependency: Some(resolved.lib_name.clone()),  // NEW
    evidence: Some(evidence_line.trim().to_string()),
});
```

### Final Dedup Pass (New, scanner.rs after line 362)
```rust
// Step 8.5: Final dedup pass before payload assembly
// Key: (source_file, protocol, target_name)
// Priority: pattern (3) > wrapper_trace (2) > library_resolution (1) > others (0)
{
    use std::collections::HashMap;
    
    let mut dedup_map: HashMap<(String, String, String), ConnectionInfo> = HashMap::new();
    
    for conn in merged.connections {
        let key = (
            conn.source_file.clone(),
            conn.protocol.clone(),
            conn.target_name.clone(),
        );
        
        dedup_map
            .entry(key)
            .and_modify(|existing| {
                let new_score = extraction_method_score(&conn.extraction_method);
                let existing_score = extraction_method_score(&existing.extraction_method);
                if new_score > existing_score {
                    *existing = conn.clone();
                }
            })
            .or_insert(conn);
    }
    
    merged.connections = dedup_map.into_values().collect();
}

/// Score extraction method for dedup priority.
/// Higher score wins; pattern > wrapper > library_resolution > others.
fn extraction_method_score(method: &str) -> u8 {
    if method.starts_with("pattern:") {
        3
    } else if method.starts_with("wrapper_trace:") {
        2
    } else if method.starts_with("library_resolution:") {
        1
    } else {
        0  // ast_*, spec:*, etc.
    }
}
```

### Payload Assembly Connection Mapping (payload.rs, lines 170-184)
```rust
let connections: Vec<ConnectionPayload> = merged
    .connections
    .into_iter()
    .map(|conn| ConnectionPayload {
        source: conn.source_service,
        target: conn.target_name,
        protocol: conn.protocol,
        method: conn.method,
        path: conn.path,
        source_file: conn.source_file,
        confidence: confidence_str(&conn.confidence),
        extraction_method: conn.extraction_method,  // NEW
        dependency: conn.dependency,  // NEW
        evidence: conn.evidence,
    })
    .collect();
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| extraction_method computed but not serialized | Serialize extraction_method in ConnectionPayload | Phase 13 | Hub can filter connections by detection method |
| No dependency tracking | dependency field populated per source | Phase 13 | Hub can correlate connections to their dependency origins |
| Partial dedup (pattern vs wrapper only) | Full dedup by (source_file, protocol, target_name) | Phase 13 | No duplicate connections reach hub across all detection sources |

## Open Questions

1. **Wrapper Tracing Dependency Propagation**
   - What we know: Wrapper tracing can wrap a pattern-detected connection (e.g., `apiFetch` wraps `fetch` wraps `httpx.post`). When the wrapper is detected, should it inherit the seed pattern's dependency?
   - What's unclear: If wrapper has its own extraction and we want to preserve traceability, should dependency be the seed pattern's dependency or the wrapper's identity?
   - Recommendation: Propagate the seed pattern's dependency through the wrapper chain. If the wrapper wraps `pattern:py-requests`, the wrapper-traced connection should have `dependency: Some("pattern:py-requests")` or `Some("py-requests")` depending on preference. Verify with the design doc or hub requirements.

2. **Empty Target Name Handling in Library Resolution**
   - What we know: Library resolution produces connections with empty `target_name` because it only identifies protocols, not specific endpoints.
   - What's unclear: Should dedup treat (source_file, protocol, "") as a unique entry, or should empty targets be grouped differently?
   - Recommendation: Treat them as unique — include empty string in the dedup key. This allows both "library uses HTTP" and "code calls http://specific.url" to coexist.

3. **Dedup Priority for Config Spec Entries**
   - What we know: Config plugins (docker-compose, kubernetes, openapi) will be added in Phase 15 and will emit ConnectionInfo with `extraction_method: spec:{type}`.
   - What's unclear: Where does `spec:*` fall in the priority order? Higher or lower than AST plugins?
   - Recommendation: Treat spec and AST as equally low priority (0). Both are direct source analysis, neither is inferred. Preserve both if they differ on target_name.

## Environment Availability

Step 2.6 (Environment Availability Audit) **SKIPPED** — Phase 13 is purely code/structural changes with no external dependencies (no tools, services, databases, or runtimes required). All work is local Rust compilation and testing.

## Sources

### Primary (HIGH confidence)
- **Project source code** (types/mod.rs, payload.rs, scanner.rs, patterns/mod.rs, wrapper/mod.rs) — verified current implementation
- **Design document** (management/opcua-adapter/scanner-data-quality-improvements.md) — referenced for requirements and fix specifications
- **REQUIREMENTS.md** (.planning/REQUIREMENTS.md) — DQ-01, DQ-02, DQ-03 requirements
- **ROADMAP.md** (.planning/ROADMAP.md) — Phase 13 success criteria

### Secondary (MEDIUM confidence)
- **CLAUDE.md** (project instructions) — confirmed Rust tech stack and no new dependencies allowed
- **Prior phase implementations** (Phases 5-7, 8-12) — pattern of extraction_method usage across codebase

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — implementation already exists, just needs field additions
- Architecture: HIGH — clear flow verified in codebase, dedup strategy matches design doc spec
- Pitfalls: HIGH — current partial dedup and field omission are already identified in codebase and design doc
- Field dependencies: HIGH — all detection sources already set extraction_method; dependency field follows same pattern

**Research date:** 2026-04-07
**Valid until:** 2026-04-14 (7 days — stable design, no fast-moving deps)
**Rust version verified:** 1.85+ (per CLAUDE.md)
**Crate versions stable:** No new crates; phase only modifies types and logic

---

## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| DQ-01 | Scanner exposes `extraction_method` on every serialized `ConnectionPayload` | extraction_method already computed in ConnectionInfo; needs field addition to ConnectionPayload struct and mapping in payload.rs assemble() function. Format verified across pattern, wrapper, libres, AST, and config sources. |
| DQ-02 | Scanner exposes `dependency` field on `ConnectionPayload` | dependency needs field addition to ConnectionInfo and ConnectionPayload; population verified in patterns/mod.rs (pattern ID), wrapper/mod.rs (seed pattern), scanner.rs (lib name); AST/config plugins set to None. |
| DQ-03 | Final dedup pass before payload assembly with (source_file, protocol, target_name) key and priority pattern > wrapper > library_resolution | Current dedup at scanner.rs:313-354 only handles pattern vs wrapper. New dedup pass must occur after merger.merge() but before resolver.resolve() using HashMap with tuple key and extraction_method_priority scoring. |
