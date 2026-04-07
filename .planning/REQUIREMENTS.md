# Requirements — v1.2 Data Quality

**Milestone goal:** Improve the quality of connection data sent to Arcanon Hub — enabling extraction method filtering, dependency tracking, and higher target resolution across all languages and repo types.

**Source:** `management/opcua-adapter/scanner-data-quality-improvements.md` (2026-04-06)

**Projected quality lift:** ~12% → ~82% composite data quality score

---

## v1.2 Requirements

### PAYLOAD — Schema additions

- [ ] **DQ-01**: Scanner exposes `extraction_method` on every serialized `ConnectionPayload` (`pattern:{id}`, `wrapper_trace:{func}→{target}`, `library_resolution:{lib}→{proto}`, `ast_{framework}`, `spec:{type}`)
- [x] **DQ-02**: Scanner exposes `dependency` field on `ConnectionPayload` — populated from pattern ID (pattern engine), lib name (library resolution), and seed pattern dependency (wrapper tracing); `None` for AST plugins

### ENGINE — Core orchestration

- [x] **DQ-03**: Scanner performs a final dedup pass before payload assembly — key is `(source_file, protocol, target_name_or_empty)`, priority order pattern > wrapper > library_resolution; connections with distinct non-empty targets are kept even if protocol matches
- [x] **DQ-04**: Pattern engine supports `TargetExtraction::EnvDefault` strategy — when a matched arg is a variable reference, searches backward up to 20 lines for env var assignment and extracts the default value; emits `env:{VAR}` hint when default not found; CDN patterns added for py-env-getenv, py-env-environ, ts-env-process, go-env-getenv (tier 1 hint only), rs-env-var, rb-env-fetch, rb-env-bracket, java-env-value, java-env-getenv (tier 1 hint only), cs-env-config

### CONFIG — Config plugin enhancements

- [ ] **DQ-05**: `.env` plugin (`plugin/config/env.rs`) emits `ConnectionInfo` entries for env file values whose key matches connection patterns (`*_URL`, `*_HOST`, `*_ENDPOINT`, `*_ADDR`, `*_DSN`, `DATABASE_URL`, `REDIS_URL`, `AMQP_URL`, `KAFKA_BROKERS`) and whose value is URL-like
- [ ] **DQ-06**: Compose plugin (`plugin/config/compose.rs`) emits `ConnectionInfo` entries for `environment:` block values that are URL-like — source service is the compose service name, target extracted from hostname
- [ ] **DQ-07**: OpenAPI plugin (`plugin/config/openapi.rs`) parses `servers[].url` (OpenAPI 3.0) and `host + basePath` (Swagger 2.0) and stores as service metadata / emits as connection hints
- [ ] **DQ-08**: Kubernetes plugin (`plugin/config/kubernetes.rs`) parses `spec.template.spec.containers[].env` entries and emits connections for URL-like values — source is the Deployment name
- [ ] **DQ-09**: New `plugin/config/spring.rs` plugin parses `application*.properties` and `application*.yml` for Spring connection keys (`spring.datasource.url`, `spring.redis.host`, `spring.kafka.bootstrap-servers`, `spring.rabbitmq.host`, etc.) and emits connections with extracted hostnames and protocols

---

## Future Requirements

These improvements are valuable but deferred to later milestones or hub work:

- Scanner-side `crossing` classification — hub computes this from authoritative cross-repo data; scanner lacks cross-repo visibility
- `method`/`path` extraction from fetch calls — only helps REST connections with literal paths; gRPC/AMQP/OPC-UA don't have HTTP methods; endpoint↔connection correlation (hub-side) is the better path
- Helm `values.yaml` parsing — Go templating requires rendering; values files have arbitrary structure; medium effort for 30-50% coverage
- Quarkus/Micronaut properties support — extends spring.rs approach; defer until Spring plugin validates the pattern

---

## Out of Scope

- Hub-side resolution improvements (fixes 10-12 from the spec doc) — these are hub v2.1 changes, not scanner changes
- Fuzzy/Levenshtein name matching in hub — risk of false positive resolution
- Language plugin changes for connection detection — language plugins detect endpoints (routes) only; all connection detection flows through pattern engine / wrapper / libres

---

## Traceability

| REQ-ID | Phase | Notes |
|--------|-------|-------|
| DQ-01 | Phase 13 | Payload Schema and Dedup |
| DQ-02 | Phase 13 | Payload Schema and Dedup |
| DQ-03 | Phase 13 | Payload Schema and Dedup |
| DQ-04 | Phase 14 | Env Var Target Extraction |
| DQ-05 | Phase 15 | Config Plugin Enhancements |
| DQ-06 | Phase 15 | Config Plugin Enhancements |
| DQ-07 | Phase 15 | Config Plugin Enhancements |
| DQ-08 | Phase 15 | Config Plugin Enhancements |
| DQ-09 | Phase 16 | Spring Boot Plugin |
