# Phase 3: Pipeline and Config Plugins — Research

**Researched:** 2026-04-04
**Domain:** Rust config file parsing, core pipeline (merge/resolve/payload/upload), parallel execution, fault tolerance
**Confidence:** HIGH (stack and architecture are project-defined; parser crate versions verified against crates.io 2026-04-04)

---

## Summary

Phase 3 wires together the full scanning pipeline using only the eight config plugins (no AST), proving that every downstream component — merger, resolver, payload assembler, and upload module — works end-to-end before language plugins add complexity. The config plugins are the simpler half of the extraction layer: they parse structured files (YAML, JSON, `.proto`, `.graphql`) using serde deserialization rather than tree-sitter queries.

The architecture is already fully specified in `docs/architecture.md`. The primary engineering challenge in this phase is not design — it is careful, correct implementation of eight distinct file formats, the deduplication logic in `merger.rs`, path normalization in `resolver.rs`, the exact ScanPayloadV1 JSON shape, and robust retry/fallback logic in `upload/mod.rs`. The pipeline must be fault-tolerant: a corrupt YAML file, a panicking plugin, or a 429 response must never abort a scan.

**Primary recommendation:** Implement plugins bottom-up (Dockerfile → env → compose → kubernetes → openapi → proto → graphql → asyncapi), wire the pipeline in scanner.rs only after all eight plugins produce correct ExtractionResults, then integrate upload last.

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| CPLU-01 | OpenAPI plugin parses openapi/swagger JSON/YAML specs to extract endpoints, schemas, and service names | `openapiv3` crate (serde structs for OAS 3.0) + manual Swagger 2.0 serde struct; file patterns from architecture.md |
| CPLU-02 | Proto plugin parses .proto files to extract gRPC services, rpc methods, and message schemas | `protobuf-parse` 3.7.2 pure-Rust parser; parses into FileDescriptorSet without protoc binary |
| CPLU-03 | GraphQL plugin parses .graphql/.gql files to extract queries, mutations, subscriptions, and types | `apollo-parser` 0.8.5; error-resilient, spec-compliant, actively maintained by Apollo |
| CPLU-04 | AsyncAPI plugin parses asyncapi JSON/YAML to extract message channels, event schemas, and protocols | `asyncapi` 0.2.0 crate (serde structs for AsyncAPI 2.x) or direct serde_yaml_bw deserialization into custom structs |
| CPLU-05 | Compose plugin parses docker-compose YAML to extract services, depends_on connections, ports, and env vars | `serde_yaml_bw` 2.5.4 into custom Docker Compose structs; anchor/merge-key pitfall documented |
| CPLU-06 | Kubernetes plugin parses k8s manifests to extract Services, Deployments, ConfigMaps, and env vars | `serde_yaml_bw` multi-document YAML; custom structs for Deployment/Service/ConfigMap kinds |
| CPLU-07 | Dockerfile plugin detects Dockerfile/Containerfile presence as service boundary markers | File pattern glob match only — no parsing required; directory of match = service root |
| CPLU-08 | Env plugin reads .env files to populate the variable resolution chain | Line-by-line `KEY=VALUE` parse; merge order defined in architecture.md Section 8 |
| PIPE-01 | Merger deduplicates services by root_path proximity, merges endpoint lists, and aggregates connections | Normalize names (lowercase, hyphens); merge by root_path when names differ (Pitfall 5) |
| PIPE-02 | Resolver matches outbound calls to local endpoints by (method, normalized_path) within the same repo | Path normalization rules defined in architecture.md Section 9 |
| PIPE-03 | Resolver normalizes paths: `:param` and `{name}` to `{param}`, regex constraints dropped, wildcards to `{*}` | Regex substitution on path segments; rules are fully specified |
| PIPE-04 | Payload assembler produces valid ScanPayloadV1 JSON matching hub's expected format | ScanPayloadV1 shape in architecture.md Section 10; `serde_json::to_string` |
| PIPE-05 | Plugins execute in parallel using rayon (config plugins first, then language plugins) | `rayon::iter::ParallelIterator`; `par_iter()` over plugin slice |
| UPLD-01 | Scanner uploads ScanPayloadV1 via POST /api/v1/scans/upload with Bearer API key auth | `reqwest` 0.13.2 with `rustls-tls` feature; `Authorization: Bearer` header |
| UPLD-02 | Scanner retries on 429 and 5xx with exponential backoff (1s, 2s, 4s — max 3 retries) | Manual retry loop with `tokio::time::sleep`; fixed backoff: 1s, 2s, 4s |
| UPLD-03 | Scanner handles 202/400/401/409/413 response codes correctly | Match on `status.as_u16()`; 409 = exit 0 (duplicate); 400/401/413 = exit 1 |
| UPLD-04 | Scanner saves payload to timestamped JSON file when network is unreachable | `std::fs::write` to `arcanon-scan-{timestamp}.json`; use `chrono` or `std::time::SystemTime` |
| FTOL-01 | Single file parse failure logs warning and continues scanning | `Result`-based error propagation; `warn!()` on Err, continue iteration |
| FTOL-02 | Plugin crash/panic is caught, logged, and other plugins continue | `std::panic::catch_unwind(AssertUnwindSafe(|| plugin.extract(ctx)))` wrapping per-plugin call |
| FTOL-03 | No services found produces a warning but still uploads (empty findings are valid) | Check `merged.services.is_empty()`; emit `warn!()` but do not abort |
| FTOL-04 | Missing git context uses directory name and deterministic content hash with user warning | Fallback logic already researched in Phase 2; Phase 3 uses whatever Phase 2 produces |
| DETQ-01 | Every finding carries a confidence field (High/Medium/Low) based on extraction method | `Confidence` enum defined in types/mod.rs; config plugins always emit `High` for spec-derived data |
| DETQ-02 | Connection findings include evidence snippets | `.proto` import lines, compose `depends_on` entries; store as `Option<String>` in ConnectionInfo |
| DETQ-03 | Connection findings include source_file attribution (file:line format) | `relative_path:line_number` string; for config files typically just the file path |
| DETQ-04 | Spec-file schemas override source-code schemas when both exist for the same endpoint | Merger priority: spec-origin schemas win over ast-origin by matching (service_name, method, path) |
| MONO-04 | Service names and scoping can be overridden via `.arcanon.toml` [services] section | TOML parse of `[services]` section; apply overrides after merger produces initial service list |
</phase_requirements>

---

## Project Constraints (from CLAUDE.md)

- **Language**: Rust — single binary, no runtime dependencies
- **Binary size**: Target < 15MB stripped (includes all tree-sitter grammars)
- **Performance**: < 2s for 100 files, < 10s for 1,000 files, < 60s for 10,000 files
- **Memory**: < 200MB peak
- **Dependencies**: Only crates listed in `docs/architecture.md` Section 12, plus `openapiv3`, `apollo-parser`, `protobuf-parse` as needed for spec parsing
- **Protocol**: Free string — no enum for protocol field; use string constants in each plugin
- **Payload format**: Must match existing hub ScanPayloadV1 schema exactly — no hub changes
- **Hard boundary**: Plugins are synchronous (rayon); upload is async (tokio); no `tokio` imports inside `src/plugin/`
- **YAML**: Use `serde_yaml_bw` (not `serde_yaml` which is deprecated)
- **GSD workflow**: All file edits must go through a GSD command (gsd:execute-phase)

---

## Standard Stack

### Core (all already locked in STACK.md)

| Crate | Version | Purpose | Why Standard |
|-------|---------|---------|--------------|
| `serde_yaml_bw` | 2.5.4 | YAML parsing (docker-compose, Kubernetes, OpenAPI, AsyncAPI) | Project-locked; active drop-in for deprecated serde_yaml; panic-free |
| `serde_json` | 1.0.149 | ScanPayloadV1 JSON assembly and upload body | Project-locked; no alternative |
| `rayon` | 1.11.0 | Parallel plugin execution | Project-locked; CPU-bound work-stealing |
| `reqwest` | 0.13.2 | HTTP upload to hub | Project-locked; rustls-tls, no OpenSSL |
| `tokio` | 1.51.0 | Async runtime for reqwest only | Project-locked; minimal feature set |
| `anyhow` | 1.0.102 | Error handling | Project-locked |
| `tracing` | 0.1.44 | Structured logging | Project-locked |

### Spec Parsing Additions (new in Phase 3)

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `openapiv3` | 2.2.0 | OpenAPI 3.0.x spec deserialization | Provides typed serde structs for OAS 3.0 path items, operations, and schemas; 7.7M downloads; stable 2.x API |
| `apollo-parser` | 0.8.5 | GraphQL schema/query parsing | Spec-compliant, error-resilient parser from Apollo; actively maintained (2026-02-25); better than `graphql-parser` which stalled |
| `protobuf-parse` | 3.7.2 | `.proto` file parsing | Pure-Rust `.proto` parser (no protoc binary dependency); part of rust-protobuf project; 22M downloads; parses into FileDescriptorSet |

### Supporting (no new crates needed)

| Crate | Already Used | Purpose | Phase 3 Use |
|-------|-------------|---------|-------------|
| `toml` | 1.1.2 | TOML parsing | Read `.arcanon.toml` `[services]` overrides (MONO-04) |
| `serde` | 1.0.228 | Derive macros | All struct deserialization |
| `tracing-subscriber` | 0.3.23 | Log output | Already wired in Phase 1/2 |

### AsyncAPI Note

The `asyncapi` crate (0.2.0, last updated April 2022) is stale and only covers AsyncAPI 2.3.0. **Do not add it as a dependency.** Instead, write custom `serde` structs covering the AsyncAPI 2.x fields needed (channels, message schemas, protocol). The spec structure maps cleanly to `serde_yaml_bw` deserialization. This avoids a stale dependency and gives full control over what fields are extracted.

**Confidence:** HIGH — versions verified against crates.io API 2026-04-04.

### Retry Logic

Do NOT add `backon` or `tokio-retry` as dependencies. The retry requirement is fixed: max 3 retries, exact delays 1s/2s/4s, only on 429 and 5xx. Implement as a simple `for retry in 0..3` loop with `tokio::time::sleep(Duration::from_secs(1 << retry))`. External retry crates add dependency weight for behavior that is trivially expressed inline.

**Installation (additions to Cargo.toml):**

```toml
openapiv3 = "2.2"
apollo-parser = "0.8"
protobuf-parse = { version = "3.7", default-features = false }
```

---

## Architecture Patterns

### Recommended Project Structure (Phase 3 additions)

```
src/
├── core/
│   ├── merger.rs          # dedup services by root_path, merge endpoints, aggregate connections
│   ├── resolver.rs        # path normalization + (method, path) intra-repo matching
│   └── payload.rs         # map internal types → ScanPayloadV1 JSON
├── upload/
│   └── mod.rs             # reqwest POST, retry loop, response code handling, file fallback
└── plugin/
    └── config/
        ├── dockerfile.rs  # glob match → service boundary
        ├── env.rs         # KEY=VALUE line parse → VariableStore population
        ├── compose.rs     # docker-compose YAML → services + connections
        ├── kubernetes.rs  # k8s manifests YAML → services + ConfigMap env vars
        ├── openapi.rs     # OAS JSON/YAML → endpoints + schemas
        ├── proto.rs       # .proto → gRPC services + message schemas
        ├── graphql.rs     # .graphql → query/mutation/type extraction
        └── asyncapi.rs    # asyncapi JSON/YAML → channels + event schemas
```

### Pattern 1: Config Plugin Skeleton

Every config plugin implements `LanguagePlugin` with `always_run() -> bool { true }`.

```rust
// Each plugin is a zero-size struct (stateless)
pub struct OpenApiPlugin;

impl LanguagePlugin for OpenApiPlugin {
    fn name(&self) -> &str { "openapi" }

    fn file_patterns(&self) -> &[&str] {
        &["**/openapi.{json,yaml,yml}", "**/swagger.{json,yaml,yml}"]
    }

    fn always_run(&self) -> bool { true }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();
        for file in &ctx.files {
            match parse_openapi_spec(&file.content) {
                Ok(spec) => extract_from_spec(&spec, file, &mut result),
                Err(e) => {
                    warn!("openapi: failed to parse {}: {}", file.relative_path, e);
                    // FTOL-01: log and continue
                }
            }
        }
        result
    }
}
```

### Pattern 2: Plugin Parallel Execution with Panic Capture (PIPE-05, FTOL-02)

Config plugins and language plugins run in parallel. Each plugin call is wrapped in `catch_unwind` to isolate panics.

```rust
use std::panic::{self, AssertUnwindSafe};

let results: Vec<ExtractionResult> = plugins
    .par_iter()
    .filter_map(|plugin| {
        let ctx = build_context(plugin, &all_files, &vars, &root);
        match panic::catch_unwind(AssertUnwindSafe(|| plugin.extract(&ctx))) {
            Ok(result) => Some(result),
            Err(panic_val) => {
                let msg = panic_val
                    .downcast_ref::<&str>()
                    .copied()
                    .unwrap_or("unknown panic");
                error!("plugin '{}' panicked: {}", plugin.name(), msg);
                None  // FTOL-02: other plugins continue
            }
        }
    })
    .collect();
```

**Critical:** `AssertUnwindSafe` is required because `ExtractionContext` contains `Arc<VariableStore>` and `Arc<str>`, which are not automatically `UnwindSafe`. The wrapper asserts that the closure will not leave shared data in an inconsistent state on unwind — this is safe because `Arc` reference counts are still decremented on unwind.

### Pattern 3: Merger — Dedup by root_path (PIPE-01, Pitfall 5)

```rust
// Step 1: normalize name for comparison
fn normalize_service_name(name: &str) -> String {
    name.to_lowercase()
        .replace(['_', ' '], "-")
}

// Step 2: group by root_path first, then name as tiebreaker
// When two services share root_path, merge into one regardless of name differences
fn merge_services(results: Vec<ExtractionResult>) -> Vec<ServiceInfo> {
    let mut by_root: HashMap<String, ServiceInfo> = HashMap::new();

    for result in results {
        for svc in result.services {
            let key = if svc.root_path.is_empty() {
                normalize_service_name(&svc.name)
            } else {
                svc.root_path.clone()
            };

            by_root.entry(key)
                .and_modify(|existing| {
                    // Name priority: compose key > package.json > dockerfile dir > inferred
                    merge_service_metadata(existing, &svc);
                })
                .or_insert(svc);
        }
    }

    by_root.into_values().collect()
}
```

### Pattern 4: Resolver — Path Normalization (PIPE-02, PIPE-03)

```rust
fn normalize_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with(':') {
                // :param → {param}
                "{param}".to_string()
            } else if segment.starts_with('{') && segment.ends_with('}') {
                // {userId} or {id:\d+} → {param}  (strip name and constraints)
                "{param}".to_string()
            } else if segment == "*" {
                "{*}".to_string()
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}
```

### Pattern 5: Upload with Retry (UPLD-01, UPLD-02, UPLD-03, UPLD-04)

```rust
pub async fn upload(payload: &ScanPayloadV1, config: &UploadConfig) -> anyhow::Result<()> {
    let client = reqwest::Client::new();
    let body = serde_json::to_string(payload)?;

    const MAX_RETRIES: u32 = 3;
    let mut last_err = None;

    for attempt in 0..=MAX_RETRIES {
        if attempt > 0 {
            // backoff: 1s, 2s, 4s
            tokio::time::sleep(Duration::from_secs(1 << (attempt - 1))).await;
        }

        let resp = client
            .post(&config.hub_url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("Content-Type", "application/json")
            .body(body.clone())
            .send()
            .await;

        match resp {
            Err(e) => {
                // Network unreachable — save file fallback (UPLD-04)
                if attempt == MAX_RETRIES {
                    return save_payload_file(payload, &e);
                }
                last_err = Some(e);
                continue;
            }
            Ok(response) => match response.status().as_u16() {
                202 => { info!("Upload accepted"); return Ok(()); }
                409 => { info!("Duplicate scan (commit already processed)"); return Ok(()); }
                429 | 500..=599 => {
                    warn!("Retryable response {}", response.status());
                    // continue retry loop
                }
                400 => anyhow::bail!("Payload validation failed: {}", response.text().await?),
                401 => anyhow::bail!("Authentication failed — check ARCANON_API_KEY"),
                413 => anyhow::bail!("Payload too large (> 10MB)"),
                code => anyhow::bail!("Unexpected response: {}", code),
            }
        }
    }

    Err(last_err.map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("Upload failed after {} retries", MAX_RETRIES)))
}
```

### Pattern 6: ScanPayloadV1 Serde Structs (PIPE-04)

The payload must match the hub's exact JSON schema (from architecture.md Section 10).

```rust
#[derive(serde::Serialize)]
pub struct ScanPayloadV1 {
    pub version: &'static str,          // always "1.0"
    pub metadata: ScanMetadata,
    pub findings: ScanFindings,
}

#[derive(serde::Serialize)]
pub struct ScanMetadata {
    pub tool: &'static str,             // always "cli"
    pub tool_version: &'static str,     // env!("CARGO_PKG_VERSION")
    pub scan_mode: &'static str,        // always "full"
    pub repo_url: String,
    pub repo_name: String,
    pub branch: String,
    pub commit_sha: String,
    pub started_at: String,             // RFC3339 timestamp
    pub completed_at: String,
    pub files_scanned: usize,
    pub project_slug: String,
}

#[derive(serde::Serialize)]
pub struct ScanFindings {
    pub services: Vec<ServicePayload>,   // endpoints nested inside
    pub connections: Vec<ConnectionPayload>,
    pub schemas: Vec<SchemaPayload>,
    pub actors: Vec<()>,                 // always empty in v1
}
```

**Key mapping:** `EndpointInfo` items nest under their owning `ServicePayload.exposes[]` — they are NOT a flat top-level array. The payload assembler must group endpoints by `service_name`.

### Anti-Patterns to Avoid

- **Embedding tokio in plugin extract():** Plugins run on rayon threads — no async/await, no `tokio::runtime::Handle::current()` calls. The hard boundary is enforced by keeping `upload/` in a separate module that main.rs calls after rayon completes.
- **Name-only dedup in merger:** Two signals for the same service can have names `"order-service"` vs `"api"` — always merge by `root_path` when non-empty, not just by normalized name.
- **Silently dropping YAML parse failures:** Use `Result`-based parsing and log each failure with `warn!`. Silent drops (e.g., letting `serde` skip unrecognized fields) are fine, but a full parse failure must be visible.
- **Spawning a new tokio runtime for upload:** Use `#[tokio::main]` in `main.rs` and `.await` the upload call. Do not call `Runtime::new().block_on()` from inside rayon threads.
- **Using `serde_yaml` instead of `serde_yaml_bw`:** The former is deprecated and archived. Project decision is `serde_yaml_bw`.

---

## Domain-Specific File Format Notes

### OpenAPI / Swagger (CPLU-01)

OpenAPI 3.0.x structure (via `openapiv3` crate):
- Top-level: `openapi` version string, `info.title` → service name, `paths` map
- `paths`: key = path string (e.g., `"/api/v1/users/{id}"`), value = `PathItem`
- `PathItem` has fields `get`, `post`, `put`, `delete`, `patch`, `head`, `options`, `trace` (each is `Option<Operation>`)
- `Operation`: `summary`, `operationId`, `requestBody`, `responses`, `tags`
- `components.schemas`: map of schema name → `Schema` with `properties`

**Swagger 2.0 note:** Some repos still have Swagger 2.0 specs (`"swagger": "2.0"`). The `openapiv3` crate does NOT parse Swagger 2.0. Detect the version field first; for Swagger 2.0, parse with custom `serde` structs or skip with a warning. Only a partial implementation is needed for v1 (extract `basePath` + `paths` map).

**Service name:** Prefer `info.title` from the spec as the service name when no other signal exists.

**Confidence:** HIGH (OAS 3.0 is fully handled by `openapiv3`; Swagger 2.0 is a known gap documented as LOW confidence).

### Protocol Buffers / .proto (CPLU-02)

`protobuf-parse` 3.7.2 parses `.proto` files into a `FileDescriptorProto` (proto2 descriptor format) using its pure-Rust parser. Key structures:

```rust
// After parsing with protobuf_parse::Parser::new().pure().parse_and_typecheck()
// Returns protobuf::descriptor::FileDescriptorSet
for file_desc in file_set.file {
    // file_desc.service[] = Vec<ServiceDescriptorProto>
    for svc in &file_desc.service {
        let service_name = svc.name(); // e.g., "UserService"
        for method in &svc.method {
            // method.name() = "GetUser"
            // method.input_type() = ".acme.GetUserRequest"
            // method.output_type() = ".acme.GetUserResponse"
        }
    }
    // file_desc.message_type[] = Vec<DescriptorProto> (message schemas)
}
```

**Limitation:** `protobuf-parse` requires all imported `.proto` files to be resolvable. For repos that use `google/protobuf/timestamp.proto` or other well-known types, the pure Rust parser may fail on imports. Handle by: (1) catching parse errors and continuing (FTOL-01), and (2) providing well-known proto include paths if possible.

**Alternative consideration:** For repos with complex import trees, a fallback regex scan for `service X {` and `rpc Y (` patterns is acceptable at Low confidence. But this is a fallback only — `protobuf-parse` is the primary path.

### GraphQL (CPLU-03)

`apollo-parser` 0.8.5 parses GraphQL Schema Definition Language (SDL) and query documents:

```rust
use apollo_parser::Parser;

let parser = Parser::new(schema_content);
let cst = parser.parse();  // returns ParseOutput

for definition in cst.document().definitions() {
    match definition {
        Definition::ObjectTypeDefinition(obj) => {
            let name = obj.name().map(|n| n.text().to_string());
            // Check if this is Query, Mutation, Subscription type
            for field in obj.fields_definition().iter().flat_map(|f| f.field_definitions()) {
                // field.name(), field.ty() = return type
            }
        }
        Definition::SchemaDefinition(schema) => {
            // schema.root_operation_type_definitions() → query/mutation/subscription types
        }
        _ => {}
    }
}
```

**Confidence:** HIGH — `apollo-parser` is spec-compliant and handles errors gracefully (returns a CST with error nodes rather than failing).

### AsyncAPI (CPLU-04)

No production-ready typed crate. Use `serde_yaml_bw` directly with custom structs:

```rust
#[derive(serde::Deserialize)]
struct AsyncApiSpec {
    asyncapi: String,          // version e.g. "2.6.0"
    info: AsyncApiInfo,
    channels: HashMap<String, ChannelItem>,
}

#[derive(serde::Deserialize)]
struct ChannelItem {
    subscribe: Option<Operation>,
    publish: Option<Operation>,
    bindings: Option<HashMap<String, serde_json::Value>>,
}
```

The `channels` map key is the channel/topic name (e.g., `"user/signedup"`). The `publish`/`subscribe` operations contain `message` with `name`, `title`, and `payload` schema. The `bindings` field holds protocol-specific config (AMQP, MQTT, Kafka binding objects).

### Docker Compose (CPLU-05)

```rust
#[derive(serde::Deserialize)]
struct ComposeFile {
    version: Option<String>,
    services: HashMap<String, ComposeService>,
}

#[derive(serde::Deserialize, Default)]
struct ComposeService {
    image: Option<String>,
    build: Option<serde_json::Value>,  // can be string or object
    ports: Option<Vec<serde_json::Value>>,  // "3000:3000" or {target: 3000, ...}
    depends_on: Option<serde_json::Value>,  // string, list, or object (v3 condition syntax)
    environment: Option<serde_json::Value>, // list or map
}
```

**Critical `depends_on` handling:** Docker Compose v3 supports two forms:
```yaml
# List form (v2 compatible):
depends_on:
  - db
  - redis

# Map form (v3 condition syntax):
depends_on:
  db:
    condition: service_healthy
```

Deserialize `depends_on` as `serde_json::Value` and match on Array vs Object variant.

**`environment` handling:** Two forms:
```yaml
environment:
  FOO: "bar"         # map form
environment:
  - FOO=bar          # list form
```

Parse both to `HashMap<String, String>` for VariableStore contribution.

**YAML anchors (Pitfall 8):** `serde_yaml_bw` handles basic anchors and merge keys. Test against fixtures with `<<: *default-service` before declaring the compose plugin complete.

### Kubernetes (CPLU-06)

K8s YAML files can contain multiple documents (separated by `---`). Use `serde_yaml_bw`'s multi-document support:

```rust
for doc in serde_yaml_bw::Deserializer::from_str(&content) {
    let manifest: serde_json::Value = serde::Deserialize::deserialize(doc)?;
    let kind = manifest["kind"].as_str().unwrap_or("");
    match kind {
        "Deployment" => handle_deployment(&manifest, &mut result),
        "Service" => handle_service(&manifest, &mut result),
        "ConfigMap" => handle_configmap(&manifest, &mut result),
        _ => {} // ignore StatefulSet, DaemonSet, Ingress, etc. for v1
    }
}
```

**ConfigMap structure:**
```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
data:
  DB_HOST: "postgres:5432"
  API_KEY: "..."
```

The `data` field maps directly to `HashMap<String, String>` for VariableStore.

**Service (k8s kind) structure:** `metadata.name` = service name, `spec.ports[].port` = exposed port.

**Deployment:** `metadata.name`, `spec.template.spec.containers[].env[]` (for env var extraction into VariableStore).

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| OpenAPI 3.0 struct deserialization | Custom OAS parser | `openapiv3` 2.2.0 | Handles `$ref` resolution, allOf/anyOf schemas, nested components; 7.7M downloads |
| GraphQL SDL parsing | Regex over .graphql files | `apollo-parser` 0.8.5 | GraphQL SDL has optional commas, block strings, directives, extend types — regex will miss edge cases |
| .proto file parsing | Custom proto tokenizer | `protobuf-parse` 3.7.2 | proto3 syntax, nested messages, well-known imports, oneof — far too complex for a custom parser |
| HTTP retry with backoff | External retry crate | Inline 3-iteration loop | The requirement is exactly 3 retries at 1s/2s/4s — no need for a general retry library |
| YAML multi-document | Custom `---` splitter | `serde_yaml_bw::Deserializer::from_str` iterates documents natively | `---` handling edge cases, anchors crossing document boundaries, BOM characters |
| Timestamp for file naming | `chrono` dependency | `std::time::SystemTime::now()` → `UNIX_EPOCH.elapsed().as_secs()` | No new dependency needed; unix timestamp is sufficient for `arcanon-scan-{ts}.json` naming |

**Key insight:** Every config file format has at least one known ambiguity (compose `depends_on` forms, k8s multi-doc, proto import resolution, GraphQL SDL edge cases). Don't spend Phase 3 time re-learning those edge cases from scratch — use the crates that already handle them.

---

## Common Pitfalls

### Pitfall A: YAML Anchors and Multi-Document Silently Drop Data (Pitfall 8 from PITFALLS.md)
**What goes wrong:** K8s ConfigMap values disappear; compose `depends_on` from merged blocks is absent.
**Why it happens:** YAML anchors (`&anchor`, `<<: *defaults`) require anchor tracking across the parse. Multi-doc `---` boundaries reset anchor scope.
**How to avoid:** Test compose and k8s plugins against fixtures that use YAML anchors. Log a `warn!()` on any unexpected `None` from required fields rather than silently ignoring.
**Warning signs:** Expected VariableStore keys absent after parsing a known-good fixture.

### Pitfall B: Service Merger Duplicates from Multi-Signal Detection (Pitfall 5 from PITFALLS.md)
**What goes wrong:** A service appears 3-4 times in payload — once per plugin that detected it.
**Why it happens:** Dockerfile, compose, and openapi plugin independently detect the same service with different names.
**How to avoid:** Merge by `root_path` first. Only fall back to name comparison when `root_path` is empty. Normalize names before comparison: lowercase, underscores/spaces → hyphens.
**Warning signs:** `--dry-run` output shows multiple services with the same `root_path` value.

### Pitfall C: compose `depends_on` Form Mismatch Causes Panic
**What goes wrong:** Parser panics or silently skips connections when `depends_on` is the v3 condition-map form instead of the expected list form.
**Why it happens:** Docker Compose 3.x added `depends_on: { service: { condition: service_healthy } }` which many codebases use.
**How to avoid:** Deserialize `depends_on` as `serde_json::Value`; match on `Value::Array` vs `Value::Object` before extracting service names.
**Warning signs:** Fixture with condition-syntax `depends_on` produces zero connections.

### Pitfall D: protobuf-parse Import Resolution Fails on Well-Known Types
**What goes wrong:** `protobuf-parse` errors out when a `.proto` file imports `google/protobuf/timestamp.proto` or similar well-known types.
**Why it happens:** The pure Rust parser needs to resolve all imports to fully typecheck. Well-known types aren't embedded.
**How to avoid:** Catch parse errors per-file (FTOL-01). Consider using `protobuf_parse::Parser::new().pure().parse_without_typecheck()` for a best-effort parse that skips import resolution. Emit `warn!()` with a note that import resolution failed.
**Warning signs:** Any `.proto` file using `import "google/protobuf/...` fails entirely instead of parsing the service definitions it does contain.

### Pitfall E: Tokio/Rayon Deadlock (Pitfall 4 from PITFALLS.md)
**What goes wrong:** Scanner hangs after plugins complete, before upload.
**Why it happens:** Any async operation called from inside a rayon worker thread.
**How to avoid:** Hard rule — `extract()` on every plugin is synchronous. The upload module is the only code that calls `.await`. In `main.rs`, rayon completes first, then tokio upload runs. No `tokio::runtime::Handle::current()` in `src/plugin/`.
**Warning signs:** Scanner output shows "Scanning complete" but hangs; 0% CPU; no upload attempt.

### Pitfall F: EndpointInfo Not Nested Under Service in Payload
**What goes wrong:** Hub rejects payload with a 400 validation error.
**Why it happens:** The internal `ExtractionResult` has a flat `Vec<EndpointInfo>`. In `ScanPayloadV1`, endpoints are nested inside each service as `exposes[]`.
**How to avoid:** In `payload.rs`, group endpoints by `service_name` and embed them inside the matching `ServicePayload.exposes` slice. The grouping step is the conversion from the flat internal model to the nested payload model.
**Warning signs:** 400 response from hub or a JSON diff against the schema in architecture.md Section 10.

### Pitfall G: OpenAPI Swagger 2.0 Fails silently with openapiv3
**What goes wrong:** `openapiv3` returns a deserialization error on Swagger 2.0 files (which use `"swagger": "2.0"` not `"openapi": "3.x"`).
**Why it happens:** The `openapiv3` crate only supports OAS 3.0.x. Old repos commonly have swagger 2.0 specs.
**How to avoid:** Read the top-level JSON/YAML first to check the version discriminator. If `swagger: "2.0"`, parse with a minimal custom struct or skip with a logged warning: "Swagger 2.0 spec detected at {path} — partial support only."
**Warning signs:** Any file named `swagger.json` or `swagger.yaml` fails parse when run through `openapiv3`.

---

## Code Examples

### OpenAPI: Extract endpoints from `openapiv3` structs

```rust
// Source: openapiv3 docs.rs 2.2.0
use openapiv3::{OpenAPI, PathItem, ReferenceOr};

let spec: OpenAPI = serde_json::from_str(json_content)
    .or_else(|_| serde_yaml_bw::from_str(yaml_content))?;

let service_name = spec.info.title.clone();

for (path_str, path_item) in &spec.paths.paths {
    let item = match path_item {
        ReferenceOr::Item(item) => item,
        ReferenceOr::Reference { .. } => continue, // skip $ref paths
    };

    for (method, operation) in item.iter() {
        endpoints.push(EndpointInfo {
            service_name: service_name.clone(),
            method: method.to_uppercase(),
            path: path_str.clone(),
            handler: operation.operation_id.clone(),
            kind: "rest".to_string(),
            confidence: Confidence::High,
            extraction_method: "spec:openapi".to_string(),
        });
    }
}
```

### Proto: Extract gRPC services from FileDescriptorProto

```rust
// Source: protobuf-parse 3.7.2 — pure Rust parser
use protobuf_parse::Parser;

let descriptors = Parser::new()
    .pure()
    .input(&file.path)
    .parse_and_typecheck()
    .or_else(|_| Parser::new().pure().input(&file.path).parse_without_typecheck())?;

for file_desc in &descriptors.file_descriptors {
    for service in &file_desc.service {
        let svc_name = service.get_name();
        for method in service.get_method() {
            endpoints.push(EndpointInfo {
                service_name: svc_name.to_string(),
                method: "rpc".to_string(),
                path: format!("{}/{}", svc_name, method.get_name()),
                kind: "grpc".to_string(),
                confidence: Confidence::High,
                extraction_method: "spec:proto".to_string(),
                ..Default::default()
            });
        }
    }
}
```

### rayon parallel plugin execution with catch_unwind

```rust
use rayon::prelude::*;
use std::panic::{self, AssertUnwindSafe};

let results: Vec<ExtractionResult> = plugins
    .par_iter()
    .filter_map(|plugin| {
        let ctx = build_context(plugin, &all_files, Arc::clone(&vars), root.clone());
        match panic::catch_unwind(AssertUnwindSafe(|| plugin.extract(&ctx))) {
            Ok(result) => Some(result),
            Err(e) => {
                let msg = e.downcast_ref::<&str>().copied().unwrap_or("unknown");
                error!("Plugin '{}' panicked: {}", plugin.name(), msg);
                None
            }
        }
    })
    .collect();
```

### Path normalization for resolver

```rust
fn normalize_path(path: &str) -> String {
    path.split('/')
        .map(|seg| {
            if seg.starts_with(':') { "{param}".into() }
            else if seg.starts_with('{') && seg.ends_with('}') { "{param}".into() }
            else if seg == "*" { "{*}".into() }
            else { seg.to_string() }
        })
        .collect::<Vec<_>>()
        .join("/")
}
```

---

## Runtime State Inventory

Not applicable — this is a greenfield implementation phase with no rename/refactor/migration.

---

## Environment Availability

| Dependency | Required By | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| Rust/Cargo | All implementation | Yes | 1.93.1 | — |
| `protobuf-parse` (pure Rust) | CPLU-02 | Available via crates.io | 3.7.2 | Fallback: regex scan for `service`/`rpc` lines at Low confidence |
| `protoc` binary | Only if non-pure parser path used | Not checked | — | Not needed — use pure Rust parser |
| Hub API endpoint | UPLD-01 | Not available locally | — | Use `--dry-run` or `--output` for development/test |

**Missing dependencies with fallback:**
- Hub API: Use `--dry-run` mode during development; upload integration test requires a real or mocked hub endpoint.

**Missing dependencies with no fallback:**
- None. All spec parsing crates are pure Rust from crates.io with no external binary requirements (provided the pure-Rust path in `protobuf-parse` is used).

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `serde_yaml` for YAML parsing | `serde_yaml_bw` (drop-in fork) | March 2024 | Project-locked decision |
| `graphql-parser` for GraphQL | `apollo-parser` 0.8.x | 2022+ (apollo-rs launch) | apollo-parser is error-resilient and actively maintained; graphql-parser last updated Dec 2024 |
| `tokio-retry` or `backoff` for retry logic | Inline retry loop | N/A | The requirement is fixed at 3 retries at 1s/2s/4s; no library needed |
| `asyncapi` crate for AsyncAPI parsing | Direct serde deserialization with custom structs | asyncapi crate stalled in 2022 | Custom structs give full control; no stale-crate risk |

---

## Open Questions

1. **Swagger 2.0 support depth**
   - What we know: `openapiv3` only handles OAS 3.0+; Swagger 2.0 uses different structure (`basePath`, `definitions` instead of `components/schemas`)
   - What's unclear: How many repos in the expected target set still use Swagger 2.0?
   - Recommendation: Implement basic Swagger 2.0 support (endpoint extraction only, no schemas) with custom serde structs; flag it as `extraction_method: "spec:swagger2"` at High confidence. Do not skip silently.

2. **protobuf-parse import resolution for well-known types**
   - What we know: Pure Rust parser requires all imports to be resolvable; well-known Google types (`google/protobuf/*.proto`) are not embedded
   - What's unclear: Whether `parse_without_typecheck()` (if available) skips import errors while still producing service/method structure
   - Recommendation: Verify `parse_without_typecheck()` availability in 3.7.2; if not available, catch errors per-file and fallback to regex extraction of `service` and `rpc` lines.

3. **.arcanon.toml `[services]` override integration point**
   - What we know: MONO-04 requires override support; TOML struct is defined in architecture.md
   - What's unclear: Whether overrides are applied before or after merger runs
   - Recommendation: Apply service name/ignore overrides in a post-merger pass. The merger first produces the canonical merged set, then a `apply_config_overrides()` function modifies names and removes ignored services. This keeps merger logic clean and override logic separate.

---

## Sources

### Primary (HIGH confidence)
- `docs/architecture.md` — Authoritative design doc; ScanPayloadV1 schema (Section 10), plugin patterns (Section 5), resolver rules (Section 9), upload protocol (Section 11)
- `.planning/research/STACK.md` — All locked crate versions; verified against crates.io 2026-04-04
- `.planning/research/PITFALLS.md` — Domain pitfalls including merger dedup (Pitfall 5), YAML anchors (Pitfall 8), tokio/rayon deadlock (Pitfall 4)
- crates.io API (verified 2026-04-04): `openapiv3` 2.2.0, `apollo-parser` 0.8.5, `protobuf-parse` 3.7.2, `asyncapi` 0.2.0

### Secondary (MEDIUM confidence)
- [openapiv3 docs.rs](https://docs.rs/openapiv3) — PathItem::iter() method for iterating HTTP methods
- [apollo-parser docs.rs](https://docs.rs/apollo-parser/latest/apollo_parser/) — CST API for schema definitions
- [protobuf-parse docs.rs](https://docs.rs/protobuf-parse/3.0.0-alpha.7/protobuf_parse/index.html) — Parser API
- [Rust std::panic::catch_unwind](https://doc.rust-lang.org/std/panic/fn.catch_unwind.html) — Official docs for panic capture pattern

### Tertiary (LOW confidence / informational)
- WebSearch: Retry crate landscape (backon, tokio-retry) — confirmed inline loop is sufficient for fixed requirements
- WebSearch: AsyncAPI Rust ecosystem — confirmed asyncapi crate is stalled; custom struct approach is correct

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — all versions verified against crates.io 2026-04-04
- Architecture: HIGH — fully specified in docs/architecture.md; Phase 3 is implementation of a designed system
- Config plugin parsing: MEDIUM-HIGH — openapiv3 and apollo-parser are well-documented; protobuf-parse import resolution is a known gap requiring validation
- Merger/resolver logic: MEDIUM — logic rules are specified; edge cases (compose depends_on forms, multi-signal service names) have concrete prevention strategies
- Upload/retry: HIGH — requirements are explicit (3 retries, 1s/2s/4s); reqwest API is stable

**Research date:** 2026-04-04
**Valid until:** 2026-07-04 (90 days — stable crates, slow-moving spec formats)
