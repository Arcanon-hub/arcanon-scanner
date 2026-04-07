# Roadmap: Arcanon Scanner

## Milestones

- ✅ **v1.0** — Phases 1-7 (shipped 2026-04-05) — [archive](milestones/v1.0-ROADMAP.md)
- ✅ **v1.1 Detection Accuracy** — Phases 8-12 (shipped 2026-04-07) — [archive](milestones/v1.1-ROADMAP.md)
- 🔄 **v1.2 Data Quality** — Phases 13-16 (active)

## Phases

<details>
<summary>✅ v1.0 (Phases 1-7) — SHIPPED 2026-04-05</summary>

- [x] Phase 1: Foundation (4/4 plans) — completed 2026-04-04
- [x] Phase 2: Infrastructure (3/3 plans) — completed 2026-04-04
- [x] Phase 3: Pipeline and Config Plugins (5/5 plans) — completed 2026-04-04
- [x] Phase 4: Language Plugins and Hardening (8/8 plans) — completed 2026-04-04
- [x] Phase 5: Pattern Engine (5/5 plans) — completed 2026-04-05
- [x] Phase 6: Library Resolution (3/3 plans) — completed 2026-04-05
- [x] Phase 7: Wrapper Tracing (3/3 plans) — completed 2026-04-05

</details>

<details>
<summary>✅ v1.1 Detection Accuracy (Phases 8-12) — SHIPPED 2026-04-07</summary>

- [x] Phase 8: Pattern Engine Accuracy (3/3 plans) — completed 2026-04-06
- [x] Phase 9: Resolver and Tech Debt (3/3 plans) — completed 2026-04-06
- [x] Phase 10: Integration Validation (1/1 plan) — completed 2026-04-06
- [x] Phase 11: Wrapper Tracing Accuracy (1/1 plan) — completed 2026-04-06
- [x] Phase 12: Wrapper Tracing Refinement (3/3 plans) — completed 2026-04-07

</details>

**v1.2 Data Quality (Phases 13-16):**

- [ ] **Phase 13: Payload Schema and Dedup** — Add extraction_method and dependency fields; final dedup pass before assembly
- [ ] **Phase 14: Env Var Target Extraction** — EnvDefault strategy in pattern engine + CDN patterns for 9 entries across 7 languages
- [ ] **Phase 15: Config Plugin Enhancements** — Extend .env, Compose, OpenAPI, and Kubernetes plugins to emit connection data
- [ ] **Phase 16: Spring Boot Plugin** — New spring.rs plugin parsing application properties and YAML for Spring connection keys

## Phase Details

### Phase 13: Payload Schema and Dedup
**Goal**: Every connection in the payload carries extraction method and dependency metadata; no duplicate connections reach the hub
**Depends on**: Phase 12 (v1.1 completed)
**Requirements**: DQ-01, DQ-02, DQ-03
**Success Criteria** (what must be TRUE):
  1. Every serialized `ConnectionPayload` includes a non-null `extraction_method` string in the format `pattern:{id}`, `wrapper_trace:{func}→{target}`, `library_resolution:{lib}→{proto}`, `ast_{framework}`, or `spec:{type}`
  2. Every serialized `ConnectionPayload` includes a `dependency` field — populated from pattern ID, lib name, or seed pattern dependency; `null` for AST plugins
  3. Scanning a fixture with known duplicate connections (same source_file, protocol, target_name) produces one connection in the payload, not multiple
  4. When duplicates collide, the surviving connection uses the pattern-engine version over wrapper over library_resolution
  5. Unit tests cover: extraction_method population per source (pattern, wrapper, libres, AST, spec); dependency population per source; dedup priority ordering with all three collision scenarios
**Plans**: 3 plans

Plans:
- [x] 13-01-PLAN.md — Add dependency field to ConnectionInfo; populate at all emission sites (patterns, wrapper, libres, compose)
- [ ] 13-02-PLAN.md — Add extraction_method and dependency to ConnectionPayload; update assemble()
- [ ] 13-03-PLAN.md — Final dedup pass in scanner.rs with priority scoring and unit tests

### Phase 14: Env Var Target Extraction
**Goal**: The pattern engine resolves env var references to their default values so targets are concrete URLs instead of variable names
**Depends on**: Phase 13
**Requirements**: DQ-04
**Success Criteria** (what must be TRUE):
  1. When a matched connection arg is a variable reference, the engine searches back up to 20 lines and extracts the default value from the env var assignment
  2. When no default is found, the connection target is emitted as `env:{VAR}` rather than the raw variable name
  3. CDN patterns cover all 9 new entries: py-env-getenv, py-env-environ, ts-env-process, go-env-getenv, rs-env-var, rb-env-fetch, rb-env-bracket, java-env-value, java-env-getenv, cs-env-config
  4. A fixture using `os.environ.get("DATABASE_URL", "postgres://localhost/db")` produces a connection with target `postgres://localhost/db`
  5. Unit tests cover: default value extraction per language (Python, TypeScript, Rust, Ruby); env hint fallback when no default; backward scan boundary (exactly 20 lines, not 21)
**Plans**: TBD

### Phase 15: Config Plugin Enhancements
**Goal**: The .env, Compose, OpenAPI, and Kubernetes config plugins emit connection data from their respective sources
**Depends on**: Phase 13
**Requirements**: DQ-05, DQ-06, DQ-07, DQ-08
**Success Criteria** (what must be TRUE):
  1. Scanning a `.env` file with `DATABASE_URL=postgres://db.host/mydb` emits a connection with protocol `postgresql` and target `db.host`
  2. Scanning a `docker-compose.yml` with a URL-like value in an `environment:` block emits a connection sourced from the compose service name
  3. Scanning an OpenAPI 3.0 file with a `servers:` block emits server URLs as connection hints; scanning a Swagger 2.0 file with `host + basePath` does the same
  4. Scanning a Kubernetes Deployment with URL-like values in `containers[].env` emits connections sourced from the Deployment name
  5. Unit tests cover: .env key pattern matching (URL-like vs. non-URL skip); Compose env block URL extraction; OpenAPI 3.0 and Swagger 2.0 servers parsing; K8s env value extraction per key type
**Plans**: TBD
**UI hint**: no

### Phase 16: Spring Boot Plugin
**Goal**: Java/Kotlin Spring Boot projects have their datasource, cache, messaging, and broker connections detected via properties and YAML config
**Depends on**: Phase 15
**Requirements**: DQ-09
**Success Criteria** (what must be TRUE):
  1. A `spring.rs` plugin file exists under `plugin/config/` and is registered in the plugin registry
  2. Scanning `application.properties` with `spring.datasource.url=jdbc:postgresql://db.host/mydb` emits a connection with protocol `postgresql` and target `db.host`
  3. Scanning `application.yml` with `spring.redis.host: redis.host` emits a connection with protocol `redis` and target `redis.host`
  4. Spring Kafka and RabbitMQ keys (`spring.kafka.bootstrap-servers`, `spring.rabbitmq.host`) each produce correctly typed connections
  5. Unit tests cover: .properties file parsing; YAML parsing; JDBC URL hostname extraction; bootstrap-servers multi-host parsing; non-Spring keys are not emitted
**Plans**: TBD

## Progress

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1. Foundation | v1.0 | 4/4 | Complete | 2026-04-04 |
| 2. Infrastructure | v1.0 | 3/3 | Complete | 2026-04-04 |
| 3. Pipeline and Config Plugins | v1.0 | 5/5 | Complete | 2026-04-04 |
| 4. Language Plugins and Hardening | v1.0 | 8/8 | Complete | 2026-04-04 |
| 5. Pattern Engine | v1.0 | 5/5 | Complete | 2026-04-05 |
| 6. Library Resolution | v1.0 | 3/3 | Complete | 2026-04-05 |
| 7. Wrapper Tracing | v1.0 | 3/3 | Complete | 2026-04-05 |
| 8. Pattern Engine Accuracy | v1.1 | 3/3 | Complete | 2026-04-06 |
| 9. Resolver and Tech Debt | v1.1 | 3/3 | Complete | 2026-04-06 |
| 10. Integration Validation | v1.1 | 1/1 | Complete | 2026-04-06 |
| 11. Wrapper Tracing Accuracy | v1.1 | 1/1 | Complete | 2026-04-06 |
| 12. Wrapper Tracing Refinement | v1.1 | 3/3 | Complete | 2026-04-07 |
| 13. Payload Schema and Dedup | v1.2 | 1/3 | In Progress|  |
| 14. Env Var Target Extraction | v1.2 | 0/? | Not started | — |
| 15. Config Plugin Enhancements | v1.2 | 0/? | Not started | — |
| 16. Spring Boot Plugin | v1.2 | 0/? | Not started | — |
