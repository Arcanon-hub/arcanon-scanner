# Requirements: Arcanon Scanner

**Defined:** 2026-04-04
**Core Value:** Accurately detect services, endpoints, and connections across 7 languages and 8 config formats using pure static analysis, producing a complete ScanPayloadV1 for the hub.

## v1 Requirements

### CLI & Configuration

- [x] **CLI-01**: User can run `arcanon-scanner [PATH]` with sensible defaults and no config file required
- [x] **CLI-02**: User can configure scanner via `.arcanon.toml` with precedence: CLI flags > env vars > config file > defaults
- [x] **CLI-03**: User can pass `--hub-url`, `--api-key`, `--project-slug` via flags or env vars (ARCANON_HUB_URL, ARCANON_API_KEY, ARCANON_PROJECT_SLUG)
- [x] **CLI-04**: User can write payload to file with `--output <FILE>` instead of uploading
- [x] **CLI-05**: User can inspect payload without side effects using `--dry-run`
- [x] **CLI-06**: User can increase log verbosity with `-v` (info), `-vv` (debug), `-vvv` (trace)
- [x] **CLI-07**: User can print version with `--version`
- [x] **CLI-08**: User can filter plugins with `--plugins <LIST>` (comma-separated)
- [x] **CLI-09**: User can add exclude patterns with `--exclude <GLOB>` (repeatable)
- [x] **CLI-10**: User can override git detection with `--repo-url`, `--branch`, `--commit-sha`
- [x] **CLI-11**: Scanner exits 0 on success, 1 on upload failure after retries, 2 on invalid arguments

### File Discovery

- [ ] **DISC-01**: Scanner walks directories using ignore crate, respecting nested .gitignore files at every level
- [ ] **DISC-02**: Scanner applies built-in excludes (.git/, node_modules/, __pycache__/, target/, dist/, build/, .next/, vendor/)
- [ ] **DISC-03**: Scanner skips files exceeding 500KB, lines exceeding 10,000 chars, and binary files (null bytes in first 8KB)
- [ ] **DISC-04**: Scanner applies additional exclude patterns from `.arcanon.toml` and `--exclude` flags
- [ ] **DISC-05**: Scanner does not follow symlinks

### Git Context

- [ ] **GIT-01**: Scanner detects repo URL from first remote (origin preferred) using gix
- [ ] **GIT-02**: Scanner detects branch from HEAD ref, falling back to CI env vars (GITHUB_REF_NAME, CI_COMMIT_BRANCH, BRANCH_NAME), then "detached"
- [ ] **GIT-03**: Scanner detects commit SHA from HEAD, falling back to CI env vars, then deterministic content hash
- [ ] **GIT-04**: Scanner derives repo_name from remote URL basename minus .git suffix, falling back to directory name

### Variable Resolution

- [ ] **VARS-01**: Scanner builds VariableStore from .env files (merge order: .env < .env.local < .env.development < .env.production)
- [ ] **VARS-02**: Scanner reads docker-compose environment entries into VariableStore
- [ ] **VARS-03**: Scanner reads Kubernetes ConfigMap data into VariableStore
- [ ] **VARS-04**: Language plugins can resolve variable names through the store and extract service targets from URL values
- [ ] **VARS-05**: Scanner traces variable references through: inline literal, local constant, module-level constant (one import level), .env, compose env, k8s ConfigMap, env var name heuristic

### Config Plugins

- [x] **CPLU-01**: OpenAPI plugin parses openapi/swagger JSON/YAML specs to extract endpoints, schemas, and service names
- [x] **CPLU-02**: Proto plugin parses .proto files to extract gRPC services, rpc methods, and message schemas
- [x] **CPLU-03**: GraphQL plugin parses .graphql/.gql files to extract queries, mutations, subscriptions, and types
- [x] **CPLU-04**: AsyncAPI plugin parses asyncapi JSON/YAML to extract message channels, event schemas, and protocols
- [x] **CPLU-05**: Compose plugin parses docker-compose YAML to extract services, depends_on connections, ports, and env vars
- [x] **CPLU-06**: Kubernetes plugin parses k8s manifests to extract Services, Deployments, ConfigMaps, and env vars
- [x] **CPLU-07**: Dockerfile plugin detects Dockerfile/Containerfile presence as service boundary markers
- [x] **CPLU-08**: Env plugin reads .env files to populate the variable resolution chain

### Language Plugins

- [ ] **LPLU-01**: TypeScript plugin detects Express, NestJS, Next.js, Fastify routes and fetch/axios/got client calls via tree-sitter AST
- [ ] **LPLU-02**: Python plugin detects FastAPI, Django, Flask routes and httpx/requests/aiohttp client calls via tree-sitter AST
- [x] **LPLU-03**: Go plugin detects net/http, Gin, Echo, Fiber routes and http.Get/grpc.Dial client calls via tree-sitter AST
- [ ] **LPLU-04**: Java plugin detects Spring Boot annotations (@RestController, @RequestMapping) and RestTemplate/WebClient calls via tree-sitter AST
- [ ] **LPLU-05**: C# plugin detects ASP.NET Core attributes ([ApiController], [HttpGet]) and HttpClient calls via tree-sitter AST
- [ ] **LPLU-06**: Rust plugin detects Actix-web, Axum, Rocket routes and reqwest/tonic client calls via tree-sitter AST
- [ ] **LPLU-07**: Ruby plugin detects Rails routes.rb, Sinatra routes, and Faraday/Net::HTTP client calls via tree-sitter AST
- [x] **LPLU-08**: Each language plugin checks framework markers (package.json, go.mod, Gemfile, etc.) before committing to full AST parsing
- [x] **LPLU-09**: Language plugins detect message queue calls (amqplib, kafkajs, mqtt.js, pika, rdkafka, rumqttc, etc.)
- [x] **LPLU-10**: Language plugins detect database client calls (pg, mongoose, redis, mysql2, sqlx, etc.) with protocol identification
- [ ] **LPLU-11**: Language plugins detect industrial protocol calls (Modbus, OPC UA, BACnet, CAN bus, HL7/FHIR) from known library patterns
- [x] **LPLU-12**: Language plugins detect gRPC client calls (grpc.Dial, ServiceStub, newBlockingStub, etc.)

### Detection Quality

- [x] **DETQ-01**: Every finding carries a confidence field (High/Medium/Low) based on extraction method
- [x] **DETQ-02**: Connection findings include evidence snippets (code fragment where the call was found)
- [x] **DETQ-03**: Connection findings include source_file attribution (file:line format)
- [x] **DETQ-04**: Spec-file schemas override source-code schemas when both exist for the same endpoint
- [ ] **DETQ-05**: Plugins that use two-phase extraction (NestJS @Controller prefix, Spring @RequestMapping, ASP.NET [Route]) produce correct full paths

### Core Pipeline

- [x] **PIPE-01**: Merger deduplicates services by root_path proximity, merges endpoint lists, and aggregates connections from all plugins
- [x] **PIPE-02**: Resolver matches outbound calls to local endpoints by (method, normalized_path) within the same repo
- [x] **PIPE-03**: Resolver normalizes paths: `:param` and `{name}` to `{param}`, regex constraints dropped, wildcards to `{*}`
- [x] **PIPE-04**: Payload assembler produces valid ScanPayloadV1 JSON matching hub's expected format
- [x] **PIPE-05**: Plugins execute in parallel using rayon (config plugins first, then language plugins)

### Monorepo Support

- [x] **MONO-01**: Scanner detects monorepos from multiple Dockerfiles, compose services, package manifests, or go.mod files
- [x] **MONO-02**: Scanner attributes source files to services using nearest-ancestor matching against service root map
- [x] **MONO-03**: Unscoped files (in shared libraries without service markers) are not attributed to any service
- [x] **MONO-04**: Service names and scoping can be overridden via `.arcanon.toml` [services] section

### Upload

- [x] **UPLD-01**: Scanner uploads ScanPayloadV1 via POST /api/v1/scans/upload with Bearer API key auth
- [x] **UPLD-02**: Scanner retries on 429 and 5xx with exponential backoff (1s, 2s, 4s — max 3 retries)
- [x] **UPLD-03**: Scanner handles 202 (success), 400 (validation error), 401 (auth error), 409 (duplicate — exit 0), 413 (too large)
- [x] **UPLD-04**: Scanner saves payload to timestamped JSON file when network is unreachable

### Fault Tolerance

- [x] **FTOL-01**: Single file parse failure logs warning and continues scanning
- [x] **FTOL-02**: Plugin crash/panic is caught, logged, and other plugins continue
- [x] **FTOL-03**: No services found produces a warning but still uploads (empty findings are valid)
- [x] **FTOL-04**: Missing git context uses directory name and deterministic content hash with user warning

### Build Tooling

- [x] **BLDG-01**: Makefile includes `lint` target running clippy with deny warnings
- [x] **BLDG-02**: Makefile includes `fmt` target running rustfmt check
- [x] **BLDG-03**: Makefile includes `test` target running cargo test
- [x] **BLDG-04**: Makefile includes `build` target for debug and release builds
- [x] **BLDG-05**: GitHub Actions workflow runs lint, fmt, test on push/PR for Linux amd64
- [x] **BLDG-06**: GitHub Actions workflow builds release binary for x86_64-unknown-linux-musl
- [x] **BLDG-07**: Release profile configured with LTO, single codegen unit, symbol stripping for < 15MB binary

### Pattern Engine

- [x] **PTRN-01**: Scanner fetches patterns from remote endpoint (https://patterns.arcanon.dev/v1/patterns.json) at startup when hub-url is configured
- [x] **PTRN-02**: Scanner caches fetched patterns to ~/.arcanon/patterns.json with ETag/Last-Modified for conditional requests
- [x] **PTRN-03**: Scanner falls back to local cache when remote is unreachable, then to embedded defaults when no cache exists
- [x] **PTRN-04**: User can define custom patterns in .arcanon.toml [[patterns]] section that override remote patterns by ID
- [x] **PTRN-05**: Pattern engine applies import_gate (content check) + match (line scan) + target_extraction to produce ConnectionInfo findings
- [x] **PTRN-06**: Pattern findings merge with compiled plugin findings through the existing merger pipeline
- [x] **PTRN-07**: ScanPayloadV1 metadata includes pattern_version and pattern_source fields

### Library Resolution

- [x] **LRES-01**: Scanner discovers Python venv (venv/, .venv/, env/) and scans library source files with the pattern engine to detect connection wrappers
- [x] **LRES-02**: Scanner discovers node_modules/ and scans library source files to detect connection wrappers
- [x] **LRES-03**: Scanner reads Cargo.lock/go.sum/Gemfile.lock transitive deps to detect libraries that depend on known connection libraries
- [x] **LRES-04**: Library resolution results are cached per-scan in a HashMap so the same library imported in multiple files is scanned only once
- [x] **LRES-05**: Missing environment (no venv, no node_modules) logs info at -v level and continues scanning with CDN patterns only
- [x] **LRES-06**: Resolved library connections have extraction_method "library_resolution:{lib}→{underlying}" and confidence Medium

### Wrapper Tracing

- [ ] **WRAP-01**: Pass 1 scans function definitions in user code and identifies wrappers around known connection functions (fetch, axios, httpx, etc.)
- [ ] **WRAP-02**: Pass 2 detects calls to discovered wrappers and extracts path/URL from arguments including template literals
- [ ] **WRAP-03**: Library wrapper detection — scans installed library source to find methods that wrap known connection functions (e.g., JournalClient.append → httpx.post)
- [ ] **WRAP-04**: Template literal path extraction normalizes interpolated segments to {param} (e.g., `/api/v1/orgs/${orgId}/teams` → `/api/v1/orgs/{param}/teams`)
- [ ] **WRAP-05**: Wrapper map chains — if function A wraps function B which wraps fetch, A is marked as a REST wrapper (multi-level)
- [ ] **WRAP-06**: Wrapper map is cached per-scan and shared across all files in the language
- [ ] **WRAP-07**: Wrapper-traced connections include extraction_method "wrapper_trace:{wrapper}→{terminal}" and the extracted path in ConnectionInfo

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
| CLI-01 | Phase 1 | Complete |
| CLI-02 | Phase 1 | Complete |
| CLI-03 | Phase 1 | Complete |
| CLI-04 | Phase 1 | Complete |
| CLI-05 | Phase 1 | Complete |
| CLI-06 | Phase 1 | Complete |
| CLI-07 | Phase 1 | Complete |
| CLI-08 | Phase 1 | Complete |
| CLI-09 | Phase 1 | Complete |
| CLI-10 | Phase 1 | Complete |
| CLI-11 | Phase 1 | Complete |
| BLDG-01 | Phase 1 | Complete |
| BLDG-02 | Phase 1 | Complete |
| BLDG-03 | Phase 1 | Complete |
| BLDG-04 | Phase 1 | Complete |
| BLDG-05 | Phase 1 | Complete |
| BLDG-06 | Phase 1 | Complete |
| BLDG-07 | Phase 1 | Complete |
| DISC-01 | Phase 2 | Complete |
| DISC-02 | Phase 2 | Complete |
| DISC-03 | Phase 2 | Complete |
| DISC-04 | Phase 2 | Complete |
| DISC-05 | Phase 2 | Complete |
| GIT-01 | Phase 2 | Complete |
| GIT-02 | Phase 2 | Complete |
| GIT-03 | Phase 2 | Complete |
| GIT-04 | Phase 2 | Complete |
| VARS-01 | Phase 2 | Complete |
| VARS-02 | Phase 2 | Complete |
| VARS-03 | Phase 2 | Complete |
| VARS-04 | Phase 2 | Complete |
| VARS-05 | Phase 2 | Complete |
| CPLU-01 | Phase 3 | Complete |
| CPLU-02 | Phase 3 | Complete |
| CPLU-03 | Phase 3 | Complete |
| CPLU-04 | Phase 3 | Complete |
| CPLU-05 | Phase 3 | Complete |
| CPLU-06 | Phase 3 | Complete |
| CPLU-07 | Phase 3 | Complete |
| CPLU-08 | Phase 3 | Complete |
| PIPE-01 | Phase 3 | Complete |
| PIPE-02 | Phase 3 | Complete |
| PIPE-03 | Phase 3 | Complete |
| PIPE-04 | Phase 3 | Complete |
| PIPE-05 | Phase 3 | Complete |
| UPLD-01 | Phase 3 | Complete |
| UPLD-02 | Phase 3 | Complete |
| UPLD-03 | Phase 3 | Complete |
| UPLD-04 | Phase 3 | Complete |
| FTOL-01 | Phase 3 | Complete |
| FTOL-02 | Phase 3 | Complete |
| FTOL-03 | Phase 3 | Complete |
| FTOL-04 | Phase 3 | Complete |
| DETQ-01 | Phase 3 | Complete |
| DETQ-02 | Phase 3 | Complete |
| DETQ-03 | Phase 3 | Complete |
| DETQ-04 | Phase 3 | Complete |
| MONO-04 | Phase 3 | Complete |
| LPLU-01 | Phase 4 | Complete |
| LPLU-02 | Phase 4 | Complete |
| LPLU-03 | Phase 4 | Complete |
| LPLU-04 | Phase 4 | Complete |
| LPLU-05 | Phase 4 | Complete |
| LPLU-06 | Phase 4 | Complete |
| LPLU-07 | Phase 4 | Complete |
| LPLU-08 | Phase 4 | Complete |
| LPLU-09 | Phase 4 | Complete |
| LPLU-10 | Phase 4 | Complete |
| LPLU-11 | Phase 4 | Complete |
| LPLU-12 | Phase 4 | Complete |
| DETQ-05 | Phase 4 | Complete |
| MONO-01 | Phase 4 | Complete |
| MONO-02 | Phase 4 | Complete |
| MONO-03 | Phase 4 | Complete |

**Coverage:**
- v1 requirements: 74 total
- Mapped to phases: 74
- Unmapped: 0

---
*Requirements defined: 2026-04-04*
*Last updated: 2026-04-04 after roadmap creation*
