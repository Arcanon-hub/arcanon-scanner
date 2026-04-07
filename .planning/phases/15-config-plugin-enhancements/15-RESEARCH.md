# Phase 15: Config Plugin Enhancements - Research

**Researched:** 2026-04-07
**Domain:** Rust config plugin extension — URL extraction from .env, Compose, OpenAPI, Kubernetes
**Confidence:** HIGH

## Summary

Phase 15 extends four existing config plugins to emit `ConnectionInfo` entries from URL-like values in their respective config sources. All four plugins share a common technical pattern: parse structured config, identify URL-like values (or known URL fields), extract hostname and protocol from those values, and emit `ConnectionInfo` entries.

The project already has all required infrastructure. The `url` crate (v2) is in `Cargo.toml` and actively used in `vars/mod.rs` via the `parse_url_to_service_target()` function. `serde_yaml_bw` is already used in both `compose.rs` and `kubernetes.rs`. The `ConnectionInfo` struct already has `extraction_method` and `dependency` fields added in Phase 13. No new crate dependencies are needed.

The four plugins can be developed in two parallel tracks: Track A covers `.env` (pure text parsing) and Compose (YAML already parsed, need to add env block traversal). Track B covers OpenAPI (spec struct already parsed, need to add servers extraction) and Kubernetes (YAML already parsed, need to add container env traversal). Within each track, the changes are independent — they do not share data structures or call each other.

**Primary recommendation:** Extract a shared `parse_url_value(val: &str) -> Option<(String, String)>` helper (returning `(protocol, hostname)`) into a new `src/plugin/config/url_util.rs` module. All four plugins call this helper. Use the existing `url` crate — no hand-rolled URL parsing.

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| DQ-05 | `.env` plugin emits `ConnectionInfo` for env keys matching connection patterns whose values are URL-like | `parse_env_file()` in vars/mod.rs already parses .env format; env plugin currently returns empty; needs to read file content and call URL util |
| DQ-06 | Compose plugin emits `ConnectionInfo` for URL-like `environment:` values; source = compose service name | `extract_compose_env()` logic in vars/mod.rs already handles list and map forms; compose.rs must iterate per-service to preserve service name |
| DQ-07 | OpenAPI plugin parses `servers[].url` (OAS3) and `host + basePath` (Swagger 2.0) as connection hints | OAS3: `openapiv3::OpenAPI` struct has `servers: Vec<ServerObject>` with `url` field; Swagger 2.0: needs `host` and `base_path` fields added to `SwaggerSpec` struct |
| DQ-08 | Kubernetes plugin emits connections for URL-like values in `containers[].env`; source = Deployment name | `K8sManifest` struct must be extended to include `spec.template.spec.containers[].env`; multi-doc iterator already in place |
</phase_requirements>

## Standard Stack

### Core (already in Cargo.toml — no additions needed)
| Crate | Version | Purpose | Notes |
|-------|---------|---------|-------|
| `url` | 2 | URL parsing — `Url::parse(val)` → hostname + scheme | [VERIFIED: Cargo.toml line 60-61]; used in vars/mod.rs |
| `serde_yaml_bw` | 2.5.4 | YAML deserialization for Compose and Kubernetes | [VERIFIED: Cargo.toml line 38] |
| `serde_json` | 1.0.149 | JSON deserialization for OpenAPI JSON files | [VERIFIED: Cargo.toml line 37] |
| `openapiv3` | 2.2 | OAS3 struct types including `ServerObject` | [VERIFIED: Cargo.toml line 69] |

**No new dependencies.** All required crates are already present.

## Architecture Patterns

### Shared URL Extraction Utility

**Create:** `src/plugin/config/url_util.rs`

This module contains two pure functions:

```rust
// Source: vars/mod.rs parse_url_to_service_target() — adapted
use url::Url;

/// Returns (protocol_string, hostname) for a URL-like value, or None if not URL-like.
/// Protocol is derived from the URL scheme using scheme_to_protocol().
pub fn parse_url_value(val: &str) -> Option<(String, String)> {
    let url = Url::parse(val).ok()?;
    let hostname = url.host_str()?.to_string();
    if hostname.is_empty() {
        return None;
    }
    let protocol = scheme_to_protocol(url.scheme());
    Some((protocol, hostname))
}

/// Map URL scheme to canonical protocol string.
/// Returns the scheme as-is for unknown schemes (free string, no enum per CLAUDE.md).
pub fn scheme_to_protocol(scheme: &str) -> String {
    match scheme {
        "postgres" | "postgresql" => "postgresql",
        "mysql" | "mariadb" => "mysql",
        "redis" | "rediss" => "redis",
        "amqp" | "amqps" => "amqp",
        "kafka" => "kafka",
        "mongodb" | "mongodb+srv" => "mongodb",
        "http" | "https" => "http",
        "grpc" | "grpcs" => "grpc",
        other => other,
    }
    .to_string()
}
```

**Register in `mod.rs`:** Add `pub mod url_util;` to `src/plugin/config/mod.rs`.

### Key Pattern Matching for .env Plugin (DQ-05)

The requirement lists explicit key patterns:
- Suffix patterns: `*_URL`, `*_HOST`, `*_ENDPOINT`, `*_ADDR`, `*_DSN`
- Exact names: `DATABASE_URL`, `REDIS_URL`, `AMQP_URL`, `KAFKA_BROKERS`

```rust
// [VERIFIED: REQUIREMENTS.md DQ-05]
fn is_connection_key(key: &str) -> bool {
    const SUFFIXES: &[&str] = &["_URL", "_HOST", "_ENDPOINT", "_ADDR", "_DSN"];
    const EXACT: &[&str] = &["DATABASE_URL", "REDIS_URL", "AMQP_URL", "KAFKA_BROKERS"];
    EXACT.contains(&key)
        || SUFFIXES.iter().any(|suf| key.ends_with(suf))
}
```

For `KAFKA_BROKERS`, the value is typically `host:port` or `host1:port,host2:port` (not a URL scheme). Parse the first comma-separated entry as hostname directly. Do not use `Url::parse()` for this key — it will fail on bare `host:port`. Detect KAFKA_BROKERS by exact name and use a separate hostname extractor.

### .env Plugin Rework (DQ-05)

`env.rs` currently returns `ExtractionResult::default()` — it is a marker plugin. The rework must:
1. Read `file.content` (already available in `ExtractionContext.files`)
2. Parse using the same logic as `parse_env_file()` in `vars/mod.rs` (duplicate the logic inline or extract to shared location)
3. Filter keys with `is_connection_key()`
4. For URL-valued keys: call `parse_url_value()`, emit `ConnectionInfo`
5. For KAFKA_BROKERS: extract hostname from `host:port` directly

`source_service` for .env connections: use the relative directory of the .env file (e.g., `services/api` from `.env` at `services/api/.env`). If at repo root, use `""` or a sentinel — match how compose.rs sets `source_service`.

`extraction_method`: `"spec:env"` — consistent with the `spec:{type}` format used by openapi (`spec:openapi`).

`dependency`: `None` — consistent with compose.rs and kubernetes.rs (config plugins set dependency to None per Phase 13 decisions).

### Compose Plugin Enhancement (DQ-06)

The existing `ComposeService` struct only has `depends_on`. It must be extended to include `environment`.

The `extract_compose_env()` function in `vars/mod.rs` already handles both YAML list form and map form but discards service names. The compose.rs enhancement must iterate per-service (not flatten all keys) to preserve the `source_service`.

```rust
// Extended struct
#[derive(Deserialize, Default)]
struct ComposeService {
    #[serde(default)]
    depends_on: DependsOn,
    #[serde(default)]
    environment: ComposeEnv,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
enum ComposeEnv {
    #[default]
    None,
    List(Vec<String>),          // ["KEY=value"]
    Map(HashMap<String, serde_yaml_bw::Value>), // {KEY: value}
}
```

For each (service_name, env entry):
- Parse key/value
- If `is_connection_key(key)` and `parse_url_value(val)` succeeds → emit `ConnectionInfo`
- `source_service`: compose service name
- `extraction_method`: `"compose"`
- `dependency`: `None`
- `source_file`: `"docker-compose.yml:0"` (same format as existing depends_on connections)

### OpenAPI Plugin Enhancement (DQ-07)

**OpenAPI 3.0:** The `openapiv3::OpenAPI` struct has a `servers` field of type `Vec<openapiv3::Server>` (or similar). Each server has a `url` field. [ASSUMED: exact field type is `Vec<openapiv3::Server>` — needs verification against openapiv3 2.2 API]

After parsing `spec` in `parse_openapi_3()`:
```rust
for server in &spec.servers {
    if let Some((protocol, hostname)) = parse_url_value(&server.url) {
        connections.push(ConnectionInfo {
            source_service: service_name.clone(),
            target_name: hostname,
            protocol,
            source_file: format!("{}:0", relative_path),
            extraction_method: "spec:openapi".to_string(),
            dependency: None,
            confidence: Confidence::Medium, // server URL is a hint, not a confirmed call
            ..Default::default()
        });
    }
}
```

**Swagger 2.0:** The existing `SwaggerSpec` struct does not have `host` or `basePath` fields. They must be added:
```rust
#[derive(Debug, Deserialize, Serialize, Default)]
struct SwaggerSpec {
    swagger: Option<String>,
    info: SwaggerInfo,
    host: Option<String>,           // ADD: e.g. "api.example.com"
    base_path: Option<String>,      // ADD: e.g. "/v1" (not used for hostname extraction)
    paths: Option<...>,
}
```

For Swagger 2.0 `host` field: it is a bare hostname (or `host:port`), not a full URL. Use `Url::parse()` on `format!("http://{}", host)` to normalize, or split on `:` to get just the hostname.

`ConnectionInfo` fields: same as OAS3. Protocol defaults to `"http"` since Swagger 2.0 `host` is scheme-agnostic (the `schemes` array indicates http/https, but parsing that is scope creep). Use `Confidence::Medium` for server hints.

The return type `ParseResult` must be extended to include `Vec<ConnectionInfo>`. Currently it returns `(Option<ServiceInfo>, Vec<EndpointInfo>, Vec<SchemaInfo>)`. The planner should widen this tuple or change to a dedicated struct.

### Kubernetes Plugin Enhancement (DQ-08)

The existing `K8sManifest` struct only captures `kind` and `metadata`. The container env array is deeply nested: `spec.template.spec.containers[].env[]`. Each env entry has:
- `name: String` — the env var key
- `value: Option<String>` — literal value (may be absent if `valueFrom:` is used)
- `valueFrom: Option<...>` — reference to ConfigMap/Secret (skip these — no literal value to parse)

Extended structs:
```rust
#[derive(Deserialize)]
struct K8sManifest {
    kind: Option<String>,
    metadata: Option<K8sMetadata>,
    spec: Option<K8sSpec>,          // ADD
}

#[derive(Deserialize)]
struct K8sSpec {
    template: Option<K8sTemplate>,
}

#[derive(Deserialize)]
struct K8sTemplate {
    spec: Option<K8sPodSpec>,
}

#[derive(Deserialize)]
struct K8sPodSpec {
    #[serde(default)]
    containers: Vec<K8sContainer>,
}

#[derive(Deserialize)]
struct K8sContainer {
    #[serde(default)]
    env: Vec<K8sEnvVar>,
}

#[derive(Deserialize)]
struct K8sEnvVar {
    name: String,
    value: Option<String>,          // literal value — present when valueFrom absent
    // valueFrom intentionally omitted — serde ignores unknown fields by default
}
```

For each container env var where `value` is `Some(val)`:
- Apply `is_connection_key(name)` + `parse_url_value(val)` → emit `ConnectionInfo`
- `source_service`: Deployment `metadata.name` (already extracted)
- `extraction_method`: `"kubernetes"`
- `dependency`: `None`
- `source_file`: `"k8s/file.yaml:0"` (same pattern as existing)
- `confidence`: `Confidence::High` (literal value in manifest is a strong signal)

### Recommended Project Structure

No new files except the shared utility:

```
src/plugin/config/
├── url_util.rs          # NEW: parse_url_value(), scheme_to_protocol(), is_connection_key()
├── mod.rs               # ADD: pub mod url_util;
├── env.rs               # MODIFY: add URL extraction logic
├── compose.rs           # MODIFY: extend ComposeService, add env extraction
├── openapi.rs           # MODIFY: extend parse_openapi_3 + parse_swagger_2 + ParseResult
└── kubernetes.rs        # MODIFY: extend K8sManifest chain, add env extraction
```

### Plan Grouping Strategy

All four plugins can be developed in parallel since they touch separate files. Recommended two plans:

**Plan 15-01:** `url_util.rs` + `.env` + `Compose` enhancements
- Rationale: url_util.rs is a prerequisite for all plugins; .env and compose both use simple key-value iteration; lower YAML complexity

**Plan 15-02:** `OpenAPI` + `Kubernetes` enhancements
- Rationale: Both require deeper struct extension (ParseResult tuple for openapi, nested spec structs for k8s); can reference url_util from plan 01

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| URL parsing | Custom scheme/host splitting via `split("://")` | `url::Url::parse()` | Handles edge cases: IPv6, port, credentials, encoded hosts, schemeless URLs |
| .env file parsing | New parser | Inline `parse_env_file()` logic from vars/mod.rs | Identical requirement, already battle-tested in the codebase |
| Protocol-from-scheme mapping | `HashMap` or `phf` lookup table | Simple `match` arm | 8 schemes — a match arm is clearest and zero-cost |

## Common Pitfalls

### Pitfall 1: EnvPlugin reads files at wrong time

**What goes wrong:** `env.rs` currently reads `ctx.files` for pattern matching but never parses content. If the enhancement reads file content from `ctx.files`, it correctly gets the pre-loaded `Arc<str>` content. Do NOT call `std::fs::read_to_string()` inside a plugin — `ctx.files` already has the content.

**How to avoid:** Use `file.content` (the `Arc<str>`) — never re-read from filesystem inside a plugin.

### Pitfall 2: Compose env — list form vs. map form

**What goes wrong:** The Compose `environment:` block has two YAML forms: list (`["KEY=val"]`) and map (`{KEY: val}`). Only handling one form misses real-world compose files.

**How to avoid:** Use `#[serde(untagged)]` enum as shown in the ComposeEnv pattern above. The existing `DependsOn` enum in compose.rs uses this exact pattern.

### Pitfall 3: OpenAPI ParseResult tuple widening

**What goes wrong:** `ParseResult` is a type alias for a 3-tuple. Adding `Vec<ConnectionInfo>` requires changing the return type and all call sites inside `extract()`. If done partially, the compiler error will catch it — but the planner must account for this change propagating to the `extract()` method body.

**How to avoid:** Change `ParseResult` to a 4-tuple or a dedicated struct. Update both `parse_openapi_3`, `parse_swagger_2`, and the `extract()` method that calls them.

### Pitfall 4: Kubernetes `valueFrom` env vars

**What goes wrong:** Many Kubernetes env entries use `valueFrom: {configMapKeyRef: ...}` or `valueFrom: {secretKeyRef: ...}` — they have no `value` field. Attempting to parse these as literal URLs will fail silently (value is None). This is correct behavior, but test fixtures must include both forms to verify the skip.

**How to avoid:** The struct has `value: Option<String>` — only process `Some(val)` entries. Serde ignores unknown fields by default, so `valueFrom` will be silently skipped.

### Pitfall 5: KAFKA_BROKERS is not a URL

**What goes wrong:** `KAFKA_BROKERS=broker1:9092,broker2:9092` — `Url::parse()` returns `Err` on this. If the code relies solely on `parse_url_value()` returning `Some`, Kafka broker connections will be silently dropped.

**How to avoid:** In `is_connection_key()` + the extraction path, detect `KAFKA_BROKERS` (or any `*_BROKERS` key) separately and split on `,`, then `:` to get hostnames. Emit protocol `"kafka"`.

### Pitfall 6: OpenAPI servers with template variables

**What goes wrong:** OpenAPI 3.0 server URLs may contain template variables: `https://{server}/v2`. `Url::parse()` on this will produce hostname `{server}` which is not useful.

**How to avoid:** After `url.host_str()`, check if the hostname contains `{` or `}` — if so, skip it (return None from the helper).

### Pitfall 7: Swagger 2.0 host field format

**What goes wrong:** Swagger 2.0 `host` field is a bare `hostname` or `hostname:port`, not a full URL. `Url::parse("api.example.com")` fails (no scheme). Must prepend `http://` for parsing.

**How to avoid:** Use `Url::parse(&format!("http://{}", host))` for Swagger 2.0 host field specifically. The scheme prefix is only for parsing — the extracted hostname will be correct.

## Code Examples

### parse_url_value — using existing `url` crate pattern

```rust
// Source: vars/mod.rs parse_url_to_service_target() — adapted for protocol return
use url::Url;

pub fn parse_url_value(val: &str) -> Option<(String, String)> {
    let url = Url::parse(val).ok()?;
    let hostname = url.host_str()?.to_string();
    if hostname.is_empty() || hostname.contains('{') {
        return None;
    }
    let protocol = scheme_to_protocol(url.scheme());
    Some((protocol, hostname))
}
```

### Compose per-service env iteration (list form)

```rust
// [VERIFIED: existing extract_compose_env in vars/mod.rs uses same pattern]
ComposeEnv::List(seq) => {
    for item in seq {
        if let Some((k, v)) = item.split_once('=') {
            if is_connection_key(k) {
                if let Some((protocol, hostname)) = parse_url_value(v) {
                    result.connections.push(ConnectionInfo {
                        source_service: service_name.clone(),
                        target_name: hostname,
                        protocol,
                        source_file: format!("{}:0", file.relative_path),
                        extraction_method: "compose".to_string(),
                        dependency: None,
                        confidence: Confidence::High,
                        method: None,
                        path: None,
                        evidence: Some(format!("{}={}", k, v)),
                    });
                }
            }
        }
    }
}
```

### Kubernetes container env traversal

```rust
// [VERIFIED: multi-doc iterator pattern already in kubernetes.rs]
if kind == "Deployment" {
    if let Some(spec) = manifest.spec {
        if let Some(template) = spec.template {
            if let Some(pod_spec) = template.spec {
                for container in pod_spec.containers {
                    for env_var in container.env {
                        if let Some(val) = env_var.value {
                            if is_connection_key(&env_var.name) {
                                if let Some((protocol, hostname)) = parse_url_value(&val) {
                                    result.connections.push(ConnectionInfo {
                                        source_service: deployment_name.clone(),
                                        target_name: hostname,
                                        protocol,
                                        ...
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| EnvPlugin as pure marker (returns empty) | EnvPlugin emits connections from URL-valued keys | Phase 15 | .env files now contribute to service graph |
| ComposePlugin only tracks depends_on | ComposePlugin also extracts env block URLs | Phase 15 | Explicit URL env vars in compose files become connections |
| OpenAPI plugin extracts endpoints only | OpenAPI plugin also extracts server URLs as connection hints | Phase 15 | API server targets visible in dependency graph |
| Kubernetes plugin extracts service names only | Kubernetes plugin also extracts env URL values | Phase 15 | K8s-injected connection config contributes to graph |

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | `openapiv3::OpenAPI` struct has a `servers` field with items having a `.url: String` field accessible in openapiv3 2.2 | Architecture Patterns — OpenAPI | If field is named differently or gated behind a type alias, the code won't compile; verify with `cargo doc --open` or crate source |
| A2 | serde ignores unknown struct fields by default in `serde_yaml_bw` (allowing K8sEnvVar to have `valueFrom` without a matching field) | Architecture Patterns — Kubernetes | If serde_yaml_bw's `Deserialize` fails on unknown fields, K8s parsing will error; add `#[serde(deny_unknown_fields)]` absence confirms default permissive behavior |

## Open Questions

1. **openapiv3 2.2 `servers` field type**
   - What we know: openapiv3 crate is used in the project; openapi.rs compiles and works
   - What's unclear: exact Rust type of `spec.servers` in openapiv3 2.2 (likely `Vec<openapiv3::Server>` with a `url` String field)
   - Recommendation: Plan 15-02 should begin with `cargo doc --open -- openapiv3` or grep crate source to confirm the type before writing the struct access code

2. **source_service for root-level .env files**
   - What we know: compose.rs uses the compose service name; kubernetes.rs uses the Deployment name
   - What's unclear: when a `.env` file is at repo root (not scoped to a service), what should `source_service` be?
   - Recommendation: Use `""` (empty string) for root-level .env files — consistent with how `vars/mod.rs` works; the hub can handle empty source

## Environment Availability

Step 2.6: SKIPPED — phase is code-only changes to existing Rust files. All dependencies (url crate, serde_yaml_bw, openapiv3) are already in Cargo.toml. No external tools or services required.

## Sources

### Primary (HIGH confidence)
- `src/plugin/config/env.rs` — current marker-only implementation, `ExtractionResult::default()` return
- `src/plugin/config/compose.rs` — existing `DependsOn` untagged enum pattern, `ComposeService` struct, connection emit pattern
- `src/plugin/config/openapi.rs` — existing `parse_openapi_3` and `parse_swagger_2` functions, `ParseResult` type alias
- `src/plugin/config/kubernetes.rs` — existing `K8sManifest` struct, multi-doc iterator, service emit pattern
- `src/vars/mod.rs` — `parse_url_to_service_target()` using `url::Url::parse()`, `extract_compose_env()` list/map handling, `parse_env_file()` logic
- `src/types/mod.rs` — `ConnectionInfo` struct with `extraction_method: String`, `dependency: Option<String>` fields
- `Cargo.toml` — confirms `url = "2"` already present, no new dependencies needed
- `.planning/REQUIREMENTS.md` — DQ-05 through DQ-08 exact key pattern lists

### Secondary (MEDIUM confidence)
- `.planning/STATE.md` — Phase 13 decisions: `dependency: None` for config plugins; `extraction_method` format `spec:{type}`

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — all crates verified in Cargo.toml; `url` crate usage verified in vars/mod.rs
- Architecture: HIGH — patterns directly derived from existing plugin code; shared utility is a straightforward extraction
- Pitfalls: HIGH — all identified from concrete code analysis (existing struct gaps, YAML form variants, URL format edge cases)

**Research date:** 2026-04-07
**Valid until:** 2026-05-07 (stable domain — Rust crate APIs don't change between patch releases)
