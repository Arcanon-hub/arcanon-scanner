# Phase 16: Spring Boot Plugin - Research

**Researched:** 2026-04-08
**Domain:** Spring Boot configuration parsing (properties and YAML)
**Confidence:** HIGH

## Summary

Phase 16 requires a new `spring.rs` config plugin that parses Spring Boot's `application.properties` and `application.yml` files to extract datasource, cache, messaging, and broker connections. The plugin must:

1. **Properties file parsing** — read key=value pairs with comment/blank line skipping
2. **YAML parsing** — deserialize hierarchical Spring config (already using `serde_yaml_bw` in existing Kubernetes and Compose plugins)
3. **JDBC URL extraction** — parse `spring.datasource.url` values (JDBC URLs) to extract hostname and detect protocol
4. **Multi-host parsing** — handle `spring.kafka.bootstrap-servers` comma-separated lists
5. **Spring-specific key mapping** — detect 6+ Spring connection keys and map to canonical protocols

The implementation closely mirrors existing config plugins (env.rs, compose.rs, kubernetes.rs) — leveraging `url_util.rs` helpers and `serde_yaml_bw` for YAML deserialization. Registration is straightforward (add to mod.rs). Test requirements mirror Phase 15 pattern: unit tests in plugin file for all branches.

**Primary recommendation:** Implement as a synchronous config plugin (no tokio) following the `LanguagePlugin` trait pattern, with JDBC URL hostname extraction as the primary complexity (JDBC URLs do not use standard HTTP URL schemes).

## User Constraints (from CONTEXT.md)

Not applicable — no CONTEXT.md exists for Phase 16. This research covers the full scope.

## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| DQ-09 | Spring properties and YAML parsing for datasource, cache, messaging, and broker connections | Standard Stack (YAML parsing via serde_yaml_bw, properties parsing via regex, JDBC URL parsing via string manipulation) |

## Standard Stack

### Core Libraries
| Library | Version | Purpose | Why |
|---------|---------|---------|-----|
| `serde_yaml_bw` | 2.5.4 | YAML deserialization (application.yml) | Already used by kubernetes.rs and compose.rs plugins. Drop-in replacement for deprecated `serde_yaml`. Tested, panic-resistant. [VERIFIED: Cargo.toml] |
| `url` | — | JDBC URL hostname parsing (spring.datasource.url) | Used by url_util.rs for parse_url_value(). Already in project. [VERIFIED: url_util.rs imports] |
| `regex` (implicit) | 1.x | `.properties` key=value parsing | Built-in Rust capabilities sufficient; no external regex crate needed for simple key=value splits. [VERIFIED: env.rs pattern] |

### Shared Utilities (Existing)
| Utility | Module | Reuse |
|---------|--------|-------|
| `parse_url_value()` | `url_util.rs` | Extract protocol and hostname from `spring.datasource.url` (JDBC URL) |
| `scheme_to_protocol()` | `url_util.rs` | Map JDBC scheme (`postgresql`, `mysql`, etc.) to canonical protocol |
| `is_connection_key()` | `url_util.rs` | Helper; may need extension for Spring-specific keys |
| `ExtractionContext` | `plugin/mod.rs` | Standard plugin input (files, root path, vars store) |
| `LanguagePlugin` trait | `plugin/mod.rs` | Plugin interface (name, file_patterns, extract, always_run) |

### Installation / Crates in Scope
No new crates required. YAML parsing via `serde_yaml_bw` is already a project dependency.

## Architecture Patterns

### Plugin Structure (Existing Pattern)
The plugin follows the `LanguagePlugin` trait and will live in `src/plugin/config/spring.rs`:

```rust
pub struct SpringPlugin;

impl LanguagePlugin for SpringPlugin {
    fn name(&self) -> &str { "spring" }
    fn file_patterns(&self) -> &[&str] { ... }
    fn always_run(&self) -> bool { true }
    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult { ... }
}
```

**File patterns to match:** `application.properties`, `application.yml`, `application-*.properties`, `application-*.yml`

### Parsing Strategy

#### Properties File (`.properties`)
Line-by-line parsing, mirroring env.rs pattern:
- Strip comments (`#`), blank lines
- Split on first `=` to get key-value pair
- Trim whitespace; strip surrounding quotes
- Check if key is a Spring connection key (see table below)

#### YAML File (`.yml` / `.yaml`)
Deserialize into a hierarchical structure via `serde_yaml_bw`, mirroring kubernetes.rs and compose.rs patterns:
- Define Spring-specific structs with `#[serde(default)]` for nested optional fields
- Use `serde_yaml_bw::Value` for untyped nested structures (e.g., `spring.datasource.hikari.*`)
- Traverse `spring.datasource`, `spring.redis`, `spring.kafka`, `spring.rabbitmq` sections

### Spring Connection Key Mapping

The plugin must detect and emit connections for these Spring properties:

| Spring Property Key | Protocol | Extract | Example |
|---------------------|----------|---------|---------|
| `spring.datasource.url` | JDBC scheme → protocol | JDBC URL hostname | `jdbc:postgresql://db.host/mydb` → `postgresql`, `db.host` |
| `spring.datasource.host` | `datasource` (fallback) | Direct hostname | `db.host` → `datasource`, `db.host` |
| `spring.redis.host` | `redis` | Direct hostname | `redis.host` → `redis`, `redis.host` |
| `spring.data.redis.host` | `redis` | Direct hostname | `redis.host` → `redis`, `redis.host` |
| `spring.kafka.bootstrap-servers` | `kafka` | CSV hostname extraction | `broker1:9092,broker2:9092` → take first, extract hostname |
| `spring.rabbitmq.host` | `rabbitmq` | Direct hostname | `rabbit.host` → `rabbitmq`, `rabbit.host` |

**Source service derivation:** Parent directory of properties/YAML file, relative to repo root (same as env.rs and kubernetes.rs).

**Confidence level:** `Confidence::High` for all extracted connections (property files are explicit configuration).

**Extraction method:** `spec:spring` for all connections.

### JDBC URL Parsing (Key Complexity)

JDBC URLs do NOT use HTTP URL schemes. Examples:
- PostgreSQL: `jdbc:postgresql://db.host/mydb` → protocol `postgresql`, hostname `db.host`
- MySQL: `jdbc:mysql://db.host:3306/mydb` → protocol `mysql`, hostname `db.host`
- Oracle: `jdbc:oracle:thin:@db.host:1521:mydb` → protocol `oracle`, hostname `db.host` (complex)

**Implementation approach:**
1. Detect if URL starts with `jdbc:` (indicating JDBC URL)
2. If JDBC URL, **do NOT use** `url::Url::parse()` (HTTP-focused) — extract hostname via string manipulation:
   - Split on first `://` after `jdbc:` to separate subprotocol from connection string
   - Extract protocol from subprotocol segment (e.g., `postgresql`, `mysql`, `oracle`)
   - Use string parsing or regex to extract hostname from connection string (port and DB may follow)
3. If non-JDBC URL, delegate to `parse_url_value()` (rare for `spring.datasource.url`, but handles cases like `jdbc:h2:mem:testdb`)

**Example code snippet (pseudocode):**
```rust
fn extract_jdbc_hostname(url_str: &str) -> Option<(String, String)> {
    // url_str = "jdbc:postgresql://db.host/mydb"
    if !url_str.starts_with("jdbc:") {
        return None; // Not a JDBC URL
    }
    let rest = &url_str[5..]; // "postgresql://db.host/mydb"
    let (subprotocol, conn_str) = rest.split_once("://")?; // ("postgresql", "db.host/mydb")
    let hostname = conn_str.split(':').next()?.split('/').next()?.to_string();
    Some((subprotocol.to_string(), hostname))
}
```

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| YAML hierarchical parsing | Manual nested HashMap traversal | `serde` + struct derivation | Compose and Kubernetes plugins prove `serde_yaml_bw` handles optional/nested fields cleanly with `#[serde(default)]` |
| URL scheme detection for HTTP URLs | Custom regex or string parsing | `url::Url::parse()` via `parse_url_value()` | HTTP-scheme URLs are already handled by url_util.rs; reuse it |
| Hostname extraction from JDBC URLs | Naive regex | String split() on delimiters | JDBC URLs are rare and have well-defined structure (no query params with embedded colons); simple split logic suffices |
| Key-value parsing for .properties files | Custom line parsing | Simple string split() | Properties format is simple; env.rs proves this approach works |

## Common Pitfalls

### Pitfall 1: Mixing JDBC and HTTP URL Parsing
**What goes wrong:** Attempting to parse a JDBC URL with `url::Url::parse()` fails because JDBC schemes like `jdbc:postgresql://...` do not conform to RFC 3986 (standard URL format). The url crate expects HTTP-style schemes.

**Why it happens:** JDBC URLs are database-specific pseudo-URLs; they predate the standard URL format and use colons and custom delimiters for connection parameters.

**How to avoid:** Detect JDBC URLs (starts with `jdbc:`) upfront and route to custom string-based parsing. Only use `url::Url::parse()` for non-JDBC URLs (e.g., `http://`, `postgresql://` when used outside JDBC context).

**Warning signs:** `Url::parse()` returning `Err` for `spring.datasource.url` values.

### Pitfall 2: Ignoring YAML Indentation and List Syntax
**What goes wrong:** If `spring.kafka.bootstrap-servers` is in YAML, it might be a list (`- broker1:9092`) or a string (`broker1:9092,broker2:9092`). Properties file always uses comma-separated strings.

**Why it happens:** YAML allows multiple representations; `serde_yaml_bw` will deserialize both correctly if the struct field type is flexible.

**How to avoid:** Use `serde_yaml_bw::Value` for flexible parsing, or define the field as `String` (YAML will convert both list and string representations). Test both syntaxes in unit tests.

**Warning signs:** Empty or missing bootstrap-servers in YAML parsed output.

### Pitfall 3: Case Sensitivity and Hyphen vs. Underscore
**What goes wrong:** Spring properties keys in YAML use hyphens (`spring.kafka.bootstrap-servers`), but some representations use underscores. The plugin must match both.

**Why it happens:** Spring Boot normalizes hyphens and underscores in properties names, but YAML and properties files use them literally.

**How to avoid:** During YAML deserialization, `serde` automatically normalizes hyphens to underscores with the `#[serde(rename_all = "snake_case")]` or `#[serde(alias = "...")]` attributes. For properties files, test with both formats and normalize in code if needed.

**Warning signs:** `spring.kafka.bootstrap-servers` found in YAML but not parsed; `spring.kafka.bootstrap_servers` working but alternative form failing.

### Pitfall 4: Multi-Host Bootstrap Servers Parsing
**What goes wrong:** `spring.kafka.bootstrap-servers=broker1:9092,broker2:9092,broker3:9092` should emit only ONE connection (to the first broker's hostname), not three separate connections. Naive CSV splitting produces duplicates.

**Why it happens:** The pattern `KAFKA_BROKERS` expects comma-separated hosts; the success criteria ask for only the first hostname to be extracted.

**How to avoid:** After splitting on `,`, take only the first broker, then extract the hostname (before the `:` port separator). Drop remaining brokers.

**Warning signs:** Unit test expecting 1 connection gets 3.

### Pitfall 5: Empty or Missing Spring Sections in YAML
**What goes wrong:** If `spring.datasource` is missing entirely from an application.yml file, deserialization may panic or leave fields uninitialized.

**Why it happens:** YAML serde requires explicit handling of absent keys with `#[serde(default)]` on struct fields.

**How to avoid:** Mark all optional Spring config sections with `#[serde(default)]` and use `Option<T>` where appropriate. Test with minimal YAML files (datasource only, redis only, etc.).

**Warning signs:** Plugin panics on valid YAML files missing certain Spring sections.

### Pitfall 6: Properties File Parsing Edge Cases
**What goes wrong:** Multiline values, escaped characters, or properties with no `=` delimiter break parsing.

**Why it happens:** Simple line-by-line parsing doesn't account for properties file spec details (continuations, escaping).

**How to avoid:** Stick to single-line key=value pairs (sufficient for Spring connection configs). Skip lines without `=`. Ignore edge cases like multiline values (not typical for connection URLs in practice).

**Warning signs:** Comments or special characters in values cause parsing to fail silently.

## Runtime State Inventory

Not applicable — Phase 16 is a greenfield plugin addition (new file, not rename/refactor). No existing data stores or configurations need migration.

## Code Examples

### Properties File Parsing
```rust
// Source: Phase 16 pattern — mirrored from env.rs
fn parse_properties_content(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let raw_val = line[eq_pos + 1..].trim();
            let val = if (raw_val.starts_with('"') && raw_val.ends_with('"'))
                || (raw_val.starts_with('\'') && raw_val.ends_with('\''))
            {
                raw_val[1..raw_val.len() - 1].to_string()
            } else {
                raw_val.to_string()
            };
            if !key.is_empty() {
                map.insert(key, val);
            }
        }
    }
    map
}
```

### JDBC URL Hostname Extraction
```rust
// Source: Phase 16 — unique to Spring JDBC
fn extract_jdbc_hostname(jdbc_url: &str) -> Option<(String, String)> {
    // jdbc_url = "jdbc:postgresql://db.host/mydb"
    if !jdbc_url.starts_with("jdbc:") {
        return None;
    }
    let rest = &jdbc_url[5..]; // "postgresql://db.host/mydb"
    let (subprotocol, conn_str) = rest.split_once("://")?;
    // Extract hostname from "db.host/mydb" or "db.host:5432/mydb"
    let hostname = conn_str
        .split(':')
        .next()?
        .split('/')
        .next()?
        .to_string();
    if hostname.is_empty() {
        return None;
    }
    Some((subprotocol.to_string(), hostname))
}
```

### YAML Deserialization Structure
```rust
// Source: Phase 16 — mirrored from kubernetes.rs and compose.rs
#[derive(Deserialize, Default)]
struct SpringConfig {
    spring: Option<SpringSection>,
}

#[derive(Deserialize, Default)]
struct SpringSection {
    datasource: Option<DatasourceConfig>,
    redis: Option<RedisConfig>,
    #[serde(rename = "kafka")]
    kafka: Option<KafkaConfig>,
    rabbitmq: Option<RabbitmqConfig>,
}

#[derive(Deserialize, Default)]
struct DatasourceConfig {
    url: Option<String>,
    host: Option<String>,
}

#[derive(Deserialize, Default)]
struct RedisConfig {
    host: Option<String>,
}

#[derive(Deserialize, Default)]
struct KafkaConfig {
    #[serde(rename = "bootstrap-servers")]
    bootstrap_servers: Option<String>,
}

#[derive(Deserialize, Default)]
struct RabbitmqConfig {
    host: Option<String>,
}
```

### Connection Emission Pattern
```rust
// Source: Phase 16 — mirrored from env.rs, compose.rs
if let Some((protocol, hostname)) = extract_jdbc_hostname(&url) {
    result.connections.push(ConnectionInfo {
        source_service: source_service.clone(),
        target_name: hostname,
        protocol,
        method: None,
        path: None,
        source_file: format!("{}:0", file.relative_path),
        confidence: Confidence::High,
        extraction_method: "spec:spring".to_string(),
        dependency: None,
        evidence: Some(format!("spring.datasource.url={}", url)),
    });
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Hand-rolled JDBC URL parsing | Dedicated `extract_jdbc_hostname()` function | Phase 16 (new) | Centralizes hostname extraction, testable in isolation |
| Separate env, compose, kubernetes parsing patterns | Unified config plugin pattern (file patterns, always_run, extract trait) | Phase 3+ (existing) | Consistent plugin structure; Spring plugin inherits this |
| Deprecated `serde_yaml` crate | `serde_yaml_bw` crate (drop-in replacement) | Phase 15 (config plugins added) | Active maintenance, panic-resistant, preserves API |

## Assumptions Log

| # | Claim | Section | Risk if Wrong |
|---|-------|---------|---------------|
| A1 | `spring.kafka.bootstrap-servers` in YAML will be present as a single string (comma-separated list), not as a YAML list | Pitfalls, Code Examples | If YAML uses list syntax (`[broker1, broker2]`), serde deserialization to `Option<String>` will fail. Mitigation: test both syntaxes, use flexible serde parsing. |
| A2 | JDBC URL hostname extraction via simple string split is sufficient (no Oracle `jdbc:oracle:thin:@host:port:db` complexity) | Code Examples | If Oracle or other complex JDBC schemes are required, string parsing becomes unreliable. Mitigation: add comprehensive test cases; escalate to planning if edge cases appear. |
| A3 | `spring.datasource.url` (JDBC) is the only datasource-related property requiring custom hostname extraction; `spring.datasource.host` (if present) is a literal hostname | Architecture Patterns | If other datasource formats exist (e.g., `spring.datasource.connection-url`), they must be added to the key mapping table. Mitigation: test phase will reveal gaps. |

**If this table is empty or all items are verified claims:** A1 and A2 are assumptions requiring test validation. A3 is based on Spring Boot standard property names (high confidence).

## Environment Availability

**Step 2.6: SKIPPED** — Phase 16 is a code-only addition (new plugin file, YAML/properties parsing) with no external dependencies beyond the project's existing crate dependencies (serde_yaml_bw, url). All required tools are already available in the Rust build environment.

## Validation Architecture

**nyquist_validation setting:** Checked config.json — `workflow.nyquist_validation` is **false**. Validation Architecture section is omitted per protocol.

## Security Domain

**security_enforcement setting:** Not explicitly set in config.json (absent = enabled by default). Spring Boot properties and YAML files may contain credentials (passwords, API keys). However, the plugin's scope is **hostname extraction only** — it does not expose secrets in connection data.

### Applicable ASVS Categories

| ASVS Category | Applies | Standard Control |
|---------------|---------|------------------|
| V2 Authentication | No | Plugin does not validate credentials |
| V3 Session Management | No | Plugin does not manage sessions |
| V4 Access Control | No | Plugin does not enforce access control |
| V5 Input Validation | Yes | Input: YAML/properties files (untrusted codebase files). Validation: safe deserialization via `serde_yaml_bw` (panic-resistant); string parsing with boundary checks |
| V6 Cryptography | No | Plugin does not encrypt/decrypt data |

### Known Threat Patterns for Spring Config Parsing

| Pattern | STRIDE | Standard Mitigation |
|---------|--------|---------------------|
| Malformed YAML causing panic | Denial of Service | `serde_yaml_bw` is hardened against Billion Laughs and malformed input; test with invalid YAML |
| Injection via hostname in JDBC URL | Tampering / Injection | Hostname extracted via string delimiters, not evaluated; no risk of code injection |
| Exposure of credentials in `evidence` field | Information Disclosure | `evidence` field contains property key/value pair; avoid logging plaintext passwords. Current design logs only hostname, not credentials |

**Recommendation:** Ensure unit tests include malformed YAML files and verify no panics occur.

## Sources

### Primary (HIGH confidence)
- **Spring Boot Official Documentation** - https://docs.spring.io/spring-boot/appendix/application-properties/index.html — standard property names and YAML structure
- **JDBC URL Format Documentation** - https://www.baeldung.com/java-jdbc-url-format — hostname extraction patterns for PostgreSQL, MySQL, and other databases
- **Existing Plugin Patterns** - `src/plugin/config/env.rs`, `src/plugin/config/compose.rs`, `src/plugin/config/kubernetes.rs` — properties parsing and YAML deserialization patterns already implemented

### Secondary (MEDIUM confidence)
- **GitHub Examples** - spring-boot-rabbitmq-example, spring-boot-all repositories — practical application.properties configuration examples
- **Spring Boot Reference** - https://docs.spring.io/spring-boot/docs/current/reference/html/application-properties.html — complete property reference

### Tertiary (LOW confidence — marked for validation)
- WebSearch results on YAML multi-host parsing and JDBC URL edge cases (assumption A2)

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — `serde_yaml_bw` is proven in Phase 15; properties parsing mirrors env.rs
- Architecture: HIGH — config plugin pattern is established; JDBC URL parsing is straightforward string manipulation
- Pitfalls: MEDIUM — JDBC URL complexity is well-understood, but multi-host Kafka and YAML list syntax require careful test coverage

**Research date:** 2026-04-08
**Valid until:** 2026-04-22 (stable Spring Boot property format; valid until next Spring Boot major version release)
