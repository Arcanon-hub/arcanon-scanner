# Requirements: Arcanon Scanner

**Defined:** 2026-04-04
**Core Value:** Accurately detect services, endpoints, and connections across 7 languages and 8 config formats using pure static analysis, producing a complete ScanPayloadV1 for the hub.

## v1 Requirements

### CLI & Configuration

- [ ] **CLI-01**: User can run `arcanon-scanner [PATH]` with sensible defaults and no config file required
- [ ] **CLI-02**: User can configure scanner via `.arcanon.toml` with precedence: CLI flags > env vars > config file > defaults
- [ ] **CLI-03**: User can pass `--hub-url`, `--api-key`, `--project-slug` via flags or env vars (ARCANON_HUB_URL, ARCANON_API_KEY, ARCANON_PROJECT_SLUG)
- [ ] **CLI-04**: User can write payload to file with `--output <FILE>` instead of uploading
- [ ] **CLI-05**: User can inspect payload without side effects using `--dry-run`
- [ ] **CLI-06**: User can increase log verbosity with `-v` (info), `-vv` (debug), `-vvv` (trace)
- [ ] **CLI-07**: User can print version with `--version`
- [ ] **CLI-08**: User can filter plugins with `--plugins <LIST>` (comma-separated)
- [ ] **CLI-09**: User can add exclude patterns with `--exclude <GLOB>` (repeatable)
- [ ] **CLI-10**: User can override git detection with `--repo-url`, `--branch`, `--commit-sha`
- [ ] **CLI-11**: Scanner exits 0 on success, 1 on upload failure after retries, 2 on invalid arguments

### File Discovery

- [ ] **DISC-01**: Scanner walks directories using ignore crate, respecting nested .gitignore files at every level
- [ ] **DISC-02**: Scanner applies built-in excludes (.git/, node_modules/, __pycache__/, target/, dist/, build/, .next/, vendor/)
- [ ] **DISC-03**: Scanner skips files exceeding 500KB, lines exceeding 10,000 chars, and binary files (null bytes in first 8KB)
- [ ] **DISC-04**: Scanner applies additional exclude patterns from `.arcanon.toml` and `--exclude` flags
- [ ] **DISC-05**: Scanner does not follow symlinks

### Git Context

- [x] **GIT-01**: Scanner detects repo URL from first remote (origin preferred) using gix
- [x] **GIT-02**: Scanner detects branch from HEAD ref, falling back to CI env vars (GITHUB_REF_NAME, CI_COMMIT_BRANCH, BRANCH_NAME), then "detached"
- [x] **GIT-03**: Scanner detects commit SHA from HEAD, falling back to CI env vars, then deterministic content hash
- [x] **GIT-04**: Scanner derives repo_name from remote URL basename minus .git suffix, falling back to directory name

### Variable Resolution

- [ ] **VARS-01**: Scanner builds VariableStore from .env files (merge order: .env < .env.local < .env.development < .env.production)
- [ ] **VARS-02**: Scanner reads docker-compose environment entries into VariableStore
- [ ] **VARS-03**: Scanner reads Kubernetes ConfigMap data into VariableStore
- [ ] **VARS-04**: Language plugins can resolve variable names through the store and extract service targets from URL values
- [ ] **VARS-05**: Scanner traces variable references through: inline literal, local constant, module-level constant (one import level), .env, compose env, k8s ConfigMap, env var name heuristic

### Config Plugins

- [ ] **CPLU-01**: OpenAPI plugin parses openapi/swagger JSON/YAML specs to extract endpoints, schemas, and service names
- [ ] **CPLU-02**: Proto plugin parses .proto files to extract gRPC services, rpc methods, and message schemas
- [ ] **CPLU-03**: GraphQL plugin parses .graphql/.gql files to extract queries, mutations, subscriptions, and types
- [ ] **CPLU-04**: AsyncAPI plugin parses asyncapi JSON/YAML to extract message channels, event schemas, and protocols
- [ ] **CPLU-05**: Compose plugin parses docker-compose YAML to extract services, depends_on connections, ports, and env vars
- [ ] **CPLU-06**: Kubernetes plugin parses k8s manifests to extract Services, Deployments, ConfigMaps, and env vars
- [ ] **CPLU-07**: Dockerfile plugin detects Dockerfile/Containerfile presence as service boundary markers
- [ ] **CPLU-08**: Env plugin reads .env files to populate the variable resolution chain

### Language Plugins

- [ ] **LPLU-01**: TypeScript plugin detects Express, NestJS, Next.js, Fastify routes and fetch/axios/got client calls via tree-sitter AST
- [ ] **LPLU-02**: Python plugin detects FastAPI, Django, Flask routes and httpx/requests/aiohttp client calls via tree-sitter AST
- [ ] **LPLU-03**: Go plugin detects net/http, Gin, Echo, Fiber routes and http.Get/grpc.Dial client calls via tree-sitter AST
- [ ] **LPLU-04**: Java plugin detects Spring Boot annotations (@RestController, @RequestMapping) and RestTemplate/WebClient calls via tree-sitter AST
- [ ] **LPLU-05**: C# plugin detects ASP.NET Core attributes ([ApiController], [HttpGet]) and HttpClient calls via tree-sitter AST
- [ ] **LPLU-06**: Rust plugin detects Actix-web, Axum, Rocket routes and reqwest/tonic client calls via tree-sitter AST
- [ ] **LPLU-07**: Ruby plugin detects Rails routes.rb, Sinatra routes, and Faraday/Net::HTTP client calls via tree-sitter AST
- [ ] **LPLU-08**: Each language plugin checks framework markers (package.json, go.mod, Gemfile, etc.) before committing to full AST parsing
- [ ] **LPLU-09**: Language plugins detect message queue calls (amqplib, kafkajs, mqtt.js, pika, rdkafka, rumqttc, etc.)
- [ ] **LPLU-10**: Language plugins detect database client calls (pg, mongoose, redis, mysql2, sqlx, etc.) with protocol identification
- [ ] **LPLU-11**: Language plugins detect industrial protocol calls (Modbus, OPC UA, BACnet, CAN bus, HL7/FHIR) from known library patterns
- [ ] **LPLU-12**: Language plugins detect gRPC client calls (grpc.Dial, ServiceStub, newBlockingStub, etc.)

### Detection Quality

- [ ] **DETQ-01**: Every finding carries a confidence field (High/Medium/Low) based on extraction method
- [ ] **DETQ-02**: Connection findings include evidence snippets (code fragment where the call was found)
- [ ] **DETQ-03**: Connection findings include source_file attribution (file:line format)
- [ ] **DETQ-04**: Spec-file schemas override source-code schemas when both exist for the same endpoint
- [ ] **DETQ-05**: Plugins that use two-phase extraction (NestJS @Controller prefix, Spring @RequestMapping, ASP.NET [Route]) produce correct full paths

### Core Pipeline

- [ ] **PIPE-01**: Merger deduplicates services by root_path proximity, merges endpoint lists, and aggregates connections from all plugins
- [ ] **PIPE-02**: Resolver matches outbound calls to local endpoints by (method, normalized_path) within the same repo
- [ ] **PIPE-03**: Resolver normalizes paths: `:param` and `{name}` to `{param}`, regex constraints dropped, wildcards to `{*}`
- [ ] **PIPE-04**: Payload assembler produces valid ScanPayloadV1 JSON matching hub's expected format
- [ ] **PIPE-05**: Plugins execute in parallel using rayon (config plugins first, then language plugins)

### Monorepo Support

- [ ] **MONO-01**: Scanner detects monorepos from multiple Dockerfiles, compose services, package manifests, or go.mod files
- [ ] **MONO-02**: Scanner attributes source files to services using nearest-ancestor matching against service root map
- [ ] **MONO-03**: Unscoped files (in shared libraries without service markers) are not attributed to any service
- [ ] **MONO-04**: Service names and scoping can be overridden via `.arcanon.toml` [services] section

### Upload

- [ ] **UPLD-01**: Scanner uploads ScanPayloadV1 via POST /api/v1/scans/upload with Bearer API key auth
- [ ] **UPLD-02**: Scanner retries on 429 and 5xx with exponential backoff (1s, 2s, 4s — max 3 retries)
- [ ] **UPLD-03**: Scanner handles 202 (success), 400 (validation error), 401 (auth error), 409 (duplicate — exit 0), 413 (too large)
- [ ] **UPLD-04**: Scanner saves payload to timestamped JSON file when network is unreachable

### Fault Tolerance

- [ ] **FTOL-01**: Single file parse failure logs warning and continues scanning
- [ ] **FTOL-02**: Plugin crash/panic is caught, logged, and other plugins continue
- [ ] **FTOL-03**: No services found produces a warning but still uploads (empty findings are valid)
- [ ] **FTOL-04**: Missing git context uses directory name and deterministic content hash with user warning

### Build Tooling

- [ ] **BLDG-01**: Makefile includes `lint` target running clippy with deny warnings
- [ ] **BLDG-02**: Makefile includes `fmt` target running rustfmt check
- [ ] **BLDG-03**: Makefile includes `test` target running cargo test
- [ ] **BLDG-04**: Makefile includes `build` target for debug and release builds
- [ ] **BLDG-05**: GitHub Actions workflow runs lint, fmt, test on push/PR for Linux amd64
- [ ] **BLDG-06**: GitHub Actions workflow builds release binary for x86_64-unknown-linux-musl
- [ ] **BLDG-07**: Release profile configured with LTO, single codegen unit, symbol stripping for < 15MB binary

## v2 Requirements

### Enhanced Analysis

- **LLME-01**: Optional LLM post-processing to resolve ambiguous connection targets
- **LLME-02**: LLM-inferred findings tagged with extraction_method "llm" and separate confidence
- **INCR-01**: Incremental scanning (only changed files since last commit)
- **NUMS-01**: Numeric confidence scores (0.0-1.0) alongside enum values

### External Plugins

- **XPLU-01**: External plugins communicate via stdin/stdout JSON protocol
- **XPLU-02**: Community plugins in any language without recompiling scanner

### Distribution

- **DIST-01**: macOS and Windows CI builds
- **DIST-02**: Homebrew tap for macOS installation
- **DIST-03**: curl-based install script (get.arcanon.dev/scanner)

### Integration Testing

- **INTG-01**: Integration tests against real open-source repos (pinned commits)
- **INTG-02**: Accuracy metrics tracking per release (service/endpoint/connection detection rates)

## Out of Scope

| Feature | Reason |
|---------|--------|
| LLM-based analysis | Adds cloud dependency, non-determinism, cost — v2 optional enhancer |
| Vulnerability/CVE detection | That's Snyk/Trivy's domain — Arcanon is structural topology only |
| SARIF output format | SARIF is for security findings (CWEs) — ScanPayloadV1 is topology data |
| Interactive/TUI mode | Breaks CI piped workflows — non-interactive always |
| IDE plugin | Separate distribution channel, dilutes focus — v3+ |
| Daemon/watch mode | Process lifecycle complexity with no v1 signal |
| Cross-repo resolution | Hub's job — scanner resolves intra-repo only |
| Auto-fix/code generation | Read-only analysis only — out of scope for topology scanner |

## Traceability

| Requirement | Phase | Status |
|-------------|-------|--------|
| CLI-01 | Phase 1 | Pending |
| CLI-02 | Phase 1 | Pending |
| CLI-03 | Phase 1 | Pending |
| CLI-04 | Phase 1 | Pending |
| CLI-05 | Phase 1 | Pending |
| CLI-06 | Phase 1 | Pending |
| CLI-07 | Phase 1 | Pending |
| CLI-08 | Phase 1 | Pending |
| CLI-09 | Phase 1 | Pending |
| CLI-10 | Phase 1 | Pending |
| CLI-11 | Phase 1 | Pending |
| BLDG-01 | Phase 1 | Pending |
| BLDG-02 | Phase 1 | Pending |
| BLDG-03 | Phase 1 | Pending |
| BLDG-04 | Phase 1 | Pending |
| BLDG-05 | Phase 1 | Pending |
| BLDG-06 | Phase 1 | Pending |
| BLDG-07 | Phase 1 | Pending |
| DISC-01 | Phase 2 | Pending |
| DISC-02 | Phase 2 | Pending |
| DISC-03 | Phase 2 | Pending |
| DISC-04 | Phase 2 | Pending |
| DISC-05 | Phase 2 | Pending |
| GIT-01 | Phase 2 | Complete |
| GIT-02 | Phase 2 | Complete |
| GIT-03 | Phase 2 | Complete |
| GIT-04 | Phase 2 | Complete |
| VARS-01 | Phase 2 | Pending |
| VARS-02 | Phase 2 | Pending |
| VARS-03 | Phase 2 | Pending |
| VARS-04 | Phase 2 | Pending |
| VARS-05 | Phase 2 | Pending |
| CPLU-01 | Phase 3 | Pending |
| CPLU-02 | Phase 3 | Pending |
| CPLU-03 | Phase 3 | Pending |
| CPLU-04 | Phase 3 | Pending |
| CPLU-05 | Phase 3 | Pending |
| CPLU-06 | Phase 3 | Pending |
| CPLU-07 | Phase 3 | Pending |
| CPLU-08 | Phase 3 | Pending |
| PIPE-01 | Phase 3 | Pending |
| PIPE-02 | Phase 3 | Pending |
| PIPE-03 | Phase 3 | Pending |
| PIPE-04 | Phase 3 | Pending |
| PIPE-05 | Phase 3 | Pending |
| UPLD-01 | Phase 3 | Pending |
| UPLD-02 | Phase 3 | Pending |
| UPLD-03 | Phase 3 | Pending |
| UPLD-04 | Phase 3 | Pending |
| FTOL-01 | Phase 3 | Pending |
| FTOL-02 | Phase 3 | Pending |
| FTOL-03 | Phase 3 | Pending |
| FTOL-04 | Phase 3 | Pending |
| DETQ-01 | Phase 3 | Pending |
| DETQ-02 | Phase 3 | Pending |
| DETQ-03 | Phase 3 | Pending |
| DETQ-04 | Phase 3 | Pending |
| MONO-04 | Phase 3 | Pending |
| LPLU-01 | Phase 4 | Pending |
| LPLU-02 | Phase 4 | Pending |
| LPLU-03 | Phase 4 | Pending |
| LPLU-04 | Phase 4 | Pending |
| LPLU-05 | Phase 4 | Pending |
| LPLU-06 | Phase 4 | Pending |
| LPLU-07 | Phase 4 | Pending |
| LPLU-08 | Phase 4 | Pending |
| LPLU-09 | Phase 4 | Pending |
| LPLU-10 | Phase 4 | Pending |
| LPLU-11 | Phase 4 | Pending |
| LPLU-12 | Phase 4 | Pending |
| DETQ-05 | Phase 4 | Pending |
| MONO-01 | Phase 4 | Pending |
| MONO-02 | Phase 4 | Pending |
| MONO-03 | Phase 4 | Pending |

**Coverage:**
- v1 requirements: 74 total
- Mapped to phases: 74
- Unmapped: 0

---
*Requirements defined: 2026-04-04*
*Last updated: 2026-04-04 after roadmap creation*
