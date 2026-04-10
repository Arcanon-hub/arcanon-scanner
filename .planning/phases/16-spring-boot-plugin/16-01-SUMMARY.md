---
phase: 16-spring-boot-plugin
plan: "01"
subsystem: plugin/config
tags: [spring, config-plugin, properties, yaml, jdbc, kafka, rabbitmq, redis]
dependency_graph:
  requires:
    - src/plugin/config/url_util.rs
    - src/plugin/mod.rs
    - src/types/mod.rs
  provides:
    - src/plugin/config/spring.rs (SpringPlugin)
  affects:
    - src/plugin/config/mod.rs
    - src/plugin/mod.rs
tech_stack:
  added: []
  patterns:
    - serde_yaml_bw struct deserialization for hierarchical YAML
    - JDBC URL hostname extraction via string split (not url::Url::parse)
    - Properties key-value parsing mirroring env.rs pattern
    - process_spring_keys() flattening YAML and properties into unified path
key_files:
  created:
    - src/plugin/config/spring.rs
  modified:
    - src/plugin/config/mod.rs
    - src/plugin/mod.rs
decisions:
  - Use string split for JDBC URL parsing (url::Url::parse fails on jdbc: scheme)
  - Flatten YAML struct into HashMap before processing to reuse process_spring_keys()
  - Take only first broker from kafka bootstrap-servers CSV list (one connection per plugin instance)
  - protocol="datasource" as fallback for spring.datasource.host (no scheme available)
metrics:
  duration: "~15 minutes"
  completed: "2026-04-08T16:20:11Z"
  tasks_completed: 1
  tasks_total: 1
  files_changed: 3
---

# Phase 16 Plan 01: Spring Boot Plugin Summary

**One-liner:** SpringPlugin parsing application.properties and application.yml for JDBC datasource, Redis, Kafka, RabbitMQ connections using string-split JDBC extraction and serde_yaml_bw deserialization.

## Tasks Completed

| Task | Name | Commit | Files |
|------|------|--------|-------|
| 1 | Implement spring.rs — JDBC extraction + properties parsing + YAML structs | f21e676 | src/plugin/config/spring.rs (created), src/plugin/config/mod.rs, src/plugin/mod.rs |

## What Was Built

`src/plugin/config/spring.rs` — a full `LanguagePlugin` implementation for Spring Boot configuration files:

- **`SpringPlugin`** struct implementing `LanguagePlugin` with `always_run() = true`
- **File patterns:** `**/application.properties`, `**/application-*.properties`, `**/application.yml`, `**/application-*.yml`
- **`extract_jdbc_hostname()`** — parses JDBC URLs (`jdbc:postgresql://host/db`) via string split to avoid RFC 3986 failures with `url::Url::parse()`
- **`parse_properties_content()`** — key=value line parser with comment/blank skipping and quote stripping, mirroring `env.rs`
- **`process_spring_keys()`** — processes 6 Spring connection keys: `spring.datasource.url`, `spring.datasource.host`, `spring.redis.host`, `spring.data.redis.host`, `spring.kafka.bootstrap-servers`, `spring.rabbitmq.host`
- **`extract_kafka_hostname()`** — takes first broker from CSV, strips port suffix
- **YAML path** — deserializes `SpringConfig` serde struct via `serde_yaml_bw::from_str`, flattens into the same HashMap for `process_spring_keys` reuse
- **Properties path** — direct `parse_properties_content` → `process_spring_keys`
- **24 unit tests** covering all 6 key types in both file formats, JDBC edge cases, Kafka multi-host, missing YAML sections, malformed YAML (no panic)

## Test Results

```
test result: ok. 311 passed; 0 failed
```

24 new spring-specific tests + 2 pre-existing java spring marker tests = 26 tests matched the `spring` filter. Full suite: 311 unit + all integration tests pass.

## Decisions Made

1. **String split for JDBC parsing** — `url::Url::parse()` fails on `jdbc:postgresql://...` because the `jdbc:` prefix makes it non-RFC-3986. Split on `://` after stripping `jdbc:` prefix to extract subprotocol and hostname.

2. **YAML struct flatten to HashMap** — The YAML path deserializes `SpringConfig` into typed structs, then inserts matching keys into a `HashMap<String, String>` that `process_spring_keys()` consumes. This avoids duplicating the key-processing logic for YAML vs properties.

3. **`protocol="datasource"` for `spring.datasource.host`** — When only a hostname is available (no URL scheme), fall back to `"datasource"` as the protocol string. Consumers can infer more specific protocol from context.

4. **First-broker-only for Kafka** — `spring.kafka.bootstrap-servers` with multiple brokers emits exactly 1 connection (first broker hostname). This matches the `env.rs` `KAFKA_BROKERS` pattern and the plan requirement.

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None. All 6 Spring connection key types are fully wired and tested.

## Threat Flags

None — no new network endpoints or trust boundaries introduced. The plugin reads local filesystem files (same trust boundary as all other config plugins). T-16-01 mitigation (serde_yaml_bw DoS resistance) is in place via the existing `serde_yaml_bw` dependency. T-16-03 evidence field: evidence stores `key=value` verbatim for non-JDBC keys; for JDBC datasource URLs the full JDBC URL is stored — passwords embedded in JDBC URLs (e.g., `jdbc:postgresql://user:pass@host/db`) would appear in evidence. This is acceptable per T-16-03 disposition ("accept") since evidence is local scan output, not transmitted credentials.

## Self-Check: PASSED

- [x] `src/plugin/config/spring.rs` exists (706 lines)
- [x] `src/plugin/config/mod.rs` registers SpringPlugin
- [x] `src/plugin/mod.rs` includes SpringPlugin in default_plugins()
- [x] Commit f21e676 exists
- [x] All 311 tests pass
