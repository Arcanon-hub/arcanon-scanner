# Arcanon Scanner — Architecture Document

**Created:** 2026-04-04
**Status:** Design — implementation next
**Companion docs:** `scanner-no-llm-feasibility.md`, `architecture-saas-platform.md`

---

## 1. Overview

Arcanon Scanner is a Rust CLI that statically analyzes codebases to extract service boundaries, endpoints, connections, and schemas — then uploads the results to Arcanon Hub as a `ScanPayloadV1`. It runs locally on developer machines (or in CI) with zero cloud dependency and zero LLM requirement.

### Design Principles

1. **No LLM.** Pure static analysis via AST parsing + config file reading. LLM is a future optional accuracy enhancer, never a dependency.
2. **Lightweight.** Single binary, no runtime dependencies. Compiles to ~10-15MB native binary.
3. **Plugin architecture.** Each language/framework is an independent plugin. Adding a new language means adding a plugin, not modifying core logic.
4. **Hub does the matching.** Scanner extracts outbound calls and exposed endpoints separately. The hub's graph reconciler matches them across repos — the scanner doesn't need cross-repo knowledge.
5. **Confidence-tagged findings.** Every finding carries a `confidence` field so the hub can filter or flag uncertain results.
6. **Protocol as free string.** No enum — supports `rest`, `grpc`, `amqp`, `kafka`, `mqtt`, `postgresql`, `mongodb`, `redis`, `modbus`, `opcua`, `bacnet`, `hl7-fhir`, or any future protocol.

---

## 2. Configuration

### `.arcanon.toml` (Repo-Level Config)

The scanner looks for `.arcanon.toml` in the scan root directory. This file is checked into version control and shared across the team. CLI flags override config file values.

```toml
# .arcanon.toml — Arcanon Scanner configuration

[scanner]
project_slug = "acme-platform"         # default --project-slug
hub_url = "https://hub.arcanon.dev"    # default --hub-url (secrets stay in env vars)

[scanner.exclude]
# Glob patterns to exclude (in addition to built-in excludes)
paths = [
    "vendor/**",
    "legacy/**",
    "**/*.generated.ts",
]

[scanner.plugins]
# Explicitly enable/disable plugins (default: all enabled)
# disabled = ["ruby", "asyncapi"]

[services]
# Override or hint service names when auto-detection gets it wrong
# Key = directory path relative to repo root, value = service name

[services."packages/api"]
name = "api-server"                    # override auto-detected name
language = "typescript"                # hint when ambiguous

[services."packages/worker"]
name = "background-worker"

[services.shared]
# Shared libraries are not services — exclude from service detection
ignore = true

[connections]
# Manual connection declarations for things the scanner can't detect
# (e.g., runtime-only service discovery, sidecar proxies)

[[connections.manual]]
source = "api-server"
target = "auth-proxy"
protocol = "rest"
path = "/auth/verify"
confidence = "high"
```

**Precedence order:** CLI flags > environment variables > `.arcanon.toml` > built-in defaults.

**What belongs here vs. env vars:** Config that's repo-specific and safe to commit goes in `.arcanon.toml` (project slug, excludes, service overrides). Secrets and user-specific values stay in env vars (`ARCANON_API_KEY`).

---

## 3. CLI Interface

```
arcanon-scanner [OPTIONS] [PATH]

Arguments:
  [PATH]  Root directory to scan (default: current directory)

Options:
  --hub-url <URL>          Hub API endpoint (default: env ARCANON_HUB_URL)
  --api-key <KEY>          API key for upload (default: env ARCANON_API_KEY)
  --project-slug <SLUG>    Project slug for grouping (default: env ARCANON_PROJECT_SLUG)
  --output <FILE>          Write payload JSON to file instead of uploading
  --dry-run                Parse and print payload, don't upload
  --plugins <LIST>         Comma-separated plugin filter (default: all enabled)
  --exclude <GLOB>         Glob patterns to exclude (repeatable)
  --repo-url <URL>         Override git remote detection
  --branch <NAME>          Override branch detection
  --commit-sha <SHA>       Override commit SHA detection
  --verbose / -v           Increase log verbosity (repeatable: -v info, -vv debug, -vvv trace)
  --version                Print version and exit
```

### Environment Variables

CLI flags always take precedence. Env vars provide defaults for repeated use and CI pipelines.

**Required (via flag or env):**

| Variable | Purpose | Notes |
|---|---|---|
| `ARCANON_API_KEY` | Org-scoped API key for upload auth | SHA-256 hash lookup on hub |
| `ARCANON_HUB_URL` | Hub API endpoint | e.g., `https://hub.arcanon.dev` |

**Optional:**

| Variable | Purpose | Notes |
|---|---|---|
| `ARCANON_PROJECT_SLUG` | Multi-repo project grouping | Avoids passing `--project-slug` every run |

**CI overrides (auto-detected but overridable):**

| Variable | Purpose | Fallback auto-detection |
|---|---|---|
| `ARCANON_REPO_URL` | Git remote URL | `gix`: first remote (`origin` preferred) |
| `ARCANON_BRANCH` | Git branch name | `gix`: HEAD ref → `GITHUB_REF_NAME` → `CI_COMMIT_BRANCH` → `BRANCH_NAME` → `"detached"` |
| `ARCANON_COMMIT_SHA` | Git commit SHA | `gix`: HEAD commit → `GITHUB_SHA` → `CI_COMMIT_SHA` |

CI overrides exist because GitHub Actions, GitLab CI, and Jenkins all check out in detached HEAD mode. `gix` can't always resolve the branch name in that state. The scanner falls back through Arcanon env vars → common CI provider env vars → `gix` detection.

**Not environment variables (CLI flags only):**

`--output`, `--dry-run`, `--plugins`, `--exclude`, `--verbose` are per-invocation concerns, not environment defaults. Timeout (30s) and retry count (3) are hardcoded sensible defaults — no knobs exposed.

**Scan mode:** v1 always runs full scans. Incremental scanning (only changed files since last commit) is a future optimization that requires either local state (`.arcanon-cache`) or a hub query (`GET /api/v1/scans/latest?repo=X`). Not designed into v1 — full scans are fast enough (< 10s for 1,000 files with tree-sitter).

### Authentication

The scanner authenticates to the hub using an org-scoped API key with `scan:upload` scope. The key is passed via `--api-key` flag or `ARCANON_API_KEY` environment variable. The hub validates it against the `api_keys` table (SHA-256 hash lookup) and resolves the org context.

### Git Integration

On startup the scanner detects git context using `gix` (pure Rust git implementation):

| Field | Source | Fallback |
|---|---|---|
| `repo_url` | First remote URL (`origin` preferred) | None |
| `repo_name` | Basename of remote URL, minus `.git` suffix | Directory name |
| `branch` | Current HEAD ref | `"detached"` |
| `commit_sha` | HEAD commit SHA | Fallback: deterministic content hash (SHA-256 of sorted file paths + sizes) |

`commit_sha` is the idempotency key on the hub. Re-scanning the same commit is a no-op.

---

## 4. Architecture

### System Context

```
┌─────────────────────────────────────────────────────┐
│                 Developer Machine / CI               │
│                                                      │
│   ┌────────────────────┐                             │
│   │  arcanon-scanner   │                             │
│   │  ┌──────────────┐  │                             │
│   │  │  Plugin Host  │  │    ┌────────────────────┐  │
│   │  │  ┌─────────┐  │  │    │  Source Code Repo  │  │
│   │  │  │ Config   │  │  │◄───│  (files on disk)   │  │
│   │  │  │ Plugins  │  │  │    └────────────────────┘  │
│   │  │  ├─────────┤  │  │                             │
│   │  │  │Language  │  │  │                             │
│   │  │  │ Plugins  │  │  │                             │
│   │  │  └─────────┘  │  │                             │
│   │  ├──────────────┤  │                             │
│   │  │  Core Engine  │  │                             │
│   │  │  (merge +     │  │                             │
│   │  │   resolve +   │──┼──► POST /api/v1/scans/upload│
│   │  │   payload)    │  │    (Arcanon Hub)            │
│   │  └──────────────┘  │                             │
│   └────────────────────┘                             │
└─────────────────────────────────────────────────────┘
```

### Project Structure

```
arcanon-scanner/
├── Cargo.toml
├── src/
│   ├── main.rs                 ← CLI entry point (clap)
│   ├── core/
│   │   ├── mod.rs
│   │   ├── scanner.rs          ← orchestration: discover → extract → merge → resolve → payload
│   │   ├── resolver.rs         ← match outbound calls to local endpoints (intra-repo)
│   │   ├── merger.rs           ← merge ExtractionResults from all plugins, dedup
│   │   └── payload.rs          ← assemble ScanPayloadV1 JSON
│   ├── git/
│   │   └── mod.rs              ← branch, commit, remote detection (gix)
│   ├── upload/
│   │   └── mod.rs              ← HTTP POST to hub (reqwest), retry, error handling
│   ├── plugin/
│   │   ├── mod.rs              ← LanguagePlugin trait + registry
│   │   ├── config/             ← config plugins (always run)
│   │   │   ├── mod.rs
│   │   │   ├── openapi.rs      ← OpenAPI/Swagger spec parser
│   │   │   ├── proto.rs        ← .proto file parser
│   │   │   ├── graphql.rs      ← .graphql schema parser
│   │   │   ├── asyncapi.rs     ← AsyncAPI spec parser
│   │   │   ├── compose.rs      ← docker-compose.yml parser
│   │   │   ├── kubernetes.rs   ← k8s manifest parser
│   │   │   ├── dockerfile.rs   ← Dockerfile/Containerfile parser
│   │   │   └── env.rs          ← .env file parser (variable resolution source)
│   │   └── lang/               ← language plugins (run when files detected)
│   │       ├── mod.rs
│   │       ├── typescript.rs   ← Express, NestJS, Next.js, Fastify, clients
│   │       ├── python.rs       ← FastAPI, Django, Flask, clients
│   │       ├── go.rs           ← net/http, Gin, Echo, Fiber, clients
│   │       ├── java.rs         ← Spring Boot, clients
│   │       ├── csharp.rs       ← ASP.NET Core, clients
│   │       ├── rust_lang.rs    ← Actix, Axum, Rocket, clients
│   │       └── ruby.rs         ← Rails, Sinatra, clients
│   ├── ast/
│   │   └── mod.rs              ← tree-sitter wrapper (query execution, node traversal)
│   ├── vars/
│   │   └── mod.rs              ← variable resolution chain (.env → compose → k8s)
│   └── types/
│       └── mod.rs              ← FileContext, ExtractionResult, ServiceInfo, etc.
```

### File Discovery and Filtering

The scanner uses the `ignore` crate (same engine as `ripgrep`) for file walking. This respects nested `.gitignore` files at every directory level.

**Built-in excludes (always applied, cannot be overridden):**

```
.git/
node_modules/
__pycache__/
.tox/
.mypy_cache/
.pytest_cache/
target/           # Rust build output
dist/
build/
out/
.next/
vendor/           # Go vendor, Ruby vendor
```

**Built-in file guards:**

| Guard | Threshold | Reason |
|---|---|---|
| Max file size | 500 KB | Skip minified JS, bundled files, generated code |
| Max line length | 10,000 chars | Skip minified single-line files |
| Binary detection | First 8KB has null bytes | Skip compiled binaries, images, fonts |

**Additional exclude patterns from `.arcanon.toml`** and `--exclude` flags are applied on top of built-in excludes.

**Symlinks:** Not followed. The `ignore` crate's `WalkBuilder` is configured with `follow_links(false)` to prevent infinite loops from circular symlinks and double-scanning via symlinked paths.

### Execution Flow

```
main.rs
  │
  ├─ 1. Parse CLI args (clap) + load .arcanon.toml
  ├─ 2. Detect git context (gix)
  ├─ 3. Build variable store (vars/)
  │     └─ Read .env, docker-compose env:, k8s ConfigMaps
  ├─ 4. Discover files (ignore crate + built-in excludes + .gitignore + .arcanon.toml excludes)
  ├─ 5. Run config plugins (always, in parallel)
  │     └─ Each returns ExtractionResult (services, endpoints, connections, schemas)
  ├─ 6. Detect languages from file extensions
  ├─ 7. Run language plugins (only matching, in parallel)
  │     └─ Each gets FileContext + variable store, returns ExtractionResult
  ├─ 8. Merge all ExtractionResults (merger.rs)
  │     └─ Dedup services by name, merge endpoint lists, aggregate connections
  ├─ 9. Resolve intra-repo connections (resolver.rs)
  │     └─ Match outbound calls to exposed endpoints within same repo
  ├─ 10. Assemble ScanPayloadV1 (payload.rs)
  ├─ 11. Upload to hub or write to file (upload/)
  └─ 12. Print summary (services found, endpoints, connections, upload status)
```

---

## 5. Plugin Architecture

### The LanguagePlugin Trait

```rust
pub trait LanguagePlugin: Send + Sync {
    /// Human-readable plugin name (e.g., "typescript", "openapi")
    fn name(&self) -> &str;

    /// Glob patterns this plugin wants to receive
    /// Config plugins: ["**/openapi.{json,yaml,yml}", "**/swagger.{json,yaml,yml}"]
    /// Language plugins: ["**/*.ts", "**/*.tsx"]
    fn file_patterns(&self) -> &[&str];

    /// Whether this plugin should always run (config plugins) or only when files match (language plugins)
    /// Default: false (only run when file_patterns match)
    fn always_run(&self) -> bool { false }

    /// Extract findings from matched files
    /// Called once with all matching files, not per-file
    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult;
}

pub struct ExtractionContext {
    /// All files matching this plugin's patterns
    pub files: Vec<FileContext>,
    /// Variable resolution store (.env, compose, k8s values)
    pub vars: Arc<VariableStore>,
    /// Repo root absolute path
    pub root: PathBuf,
}

pub struct FileContext {
    /// Absolute path
    pub path: PathBuf,
    /// Path relative to repo root
    pub relative_path: String,
    /// File contents (read once, shared across plugins)
    pub content: Arc<str>,
}
```

### ExtractionResult

```rust
pub struct ExtractionResult {
    pub services: Vec<ServiceInfo>,
    pub endpoints: Vec<EndpointInfo>,
    pub connections: Vec<ConnectionInfo>,
    pub schemas: Vec<SchemaInfo>,
    pub actors: Vec<ActorInfo>,
}

pub struct ServiceInfo {
    pub name: String,
    pub root_path: String,        // relative to repo root
    pub language: String,
    pub service_type: String,     // "service", "frontend", "database", "broker", "external"
    pub boundary_entry: Option<String>,
    pub confidence: Confidence,
    pub extraction_method: String, // "dockerfile", "compose", "package_json", "ast", etc.
}

pub struct EndpointInfo {
    pub service_name: String,     // which service exposes this
    pub method: String,           // GET, POST, rpc, subscription, etc.
    pub path: String,             // /api/v1/users, UserService/GetUser, etc.
    pub handler: Option<String>,  // function/method name
    pub kind: String,             // "rest", "grpc", "graphql", "websocket"
    pub confidence: Confidence,
    pub extraction_method: String,
}

pub struct ConnectionInfo {
    pub source_service: String,
    pub target_name: String,      // target service name or URL pattern
    pub protocol: String,         // free string: "rest", "grpc", "amqp", "postgresql", "modbus"
    pub method: Option<String>,
    pub path: Option<String>,
    pub source_file: String,      // file:line where the call was found
    pub confidence: Confidence,
    pub extraction_method: String,
    pub evidence: Option<String>, // code snippet
}

pub struct SchemaInfo {
    pub name: String,
    pub role: String,             // "request", "response", "event"
    pub file: Option<String>,
    pub connection_ref: Option<String>,
    pub fields: Vec<FieldInfo>,
    pub confidence: Confidence,
    pub extraction_method: String,
}

pub enum Confidence {
    High,     // literal URL, spec file, explicit annotation
    Medium,   // resolved variable, pattern match, convention
    Low,      // heuristic, partial match
}
```

### Plugin Categories

**Config plugins** always run regardless of detected languages. They parse specification and infrastructure files:

| Plugin | File Patterns | Extracts |
|---|---|---|
| `openapi` | `**/openapi.{json,yaml,yml}`, `**/swagger.{json,yaml,yml}` | Endpoints, schemas, service name |
| `proto` | `**/*.proto` | gRPC services, rpc methods, message schemas |
| `graphql` | `**/*.graphql`, `**/*.gql` | Queries, mutations, subscriptions, types |
| `asyncapi` | `**/asyncapi.{json,yaml,yml}` | Message channels, event schemas, protocols |
| `compose` | `**/docker-compose*.{yml,yaml}`, `**/compose*.{yml,yaml}` | Services, depends_on connections, ports, env vars |
| `kubernetes` | `**/k8s/**/*.{yml,yaml}`, `**/manifests/**/*.{yml,yaml}` | Services, Deployments, ConfigMaps, env vars |
| `dockerfile` | `**/Dockerfile*`, `**/Containerfile*` | Service boundaries (a Dockerfile = a deployable unit) |
| `env` | `**/.env*` | Variable values for the resolution chain |

**Language plugins** run only when matching source files exist:

| Plugin | File Patterns | Frameworks Detected |
|---|---|---|
| `typescript` | `**/*.ts`, `**/*.tsx`, `**/*.js`, `**/*.jsx` | Express, NestJS, Next.js, Fastify; fetch, axios, got clients |
| `python` | `**/*.py` | FastAPI, Django, Flask; httpx, requests, aiohttp clients |
| `go` | `**/*.go` | net/http, Gin, Echo, Fiber; http.Get, grpc.Dial clients |
| `java` | `**/*.java` | Spring Boot (@RestController, @RequestMapping); RestTemplate, WebClient |
| `csharp` | `**/*.cs` | ASP.NET Core ([ApiController], [HttpGet]); HttpClient, IHttpClientFactory |
| `rust_lang` | `**/*.rs` | Actix-web, Axum, Rocket; reqwest, tonic clients |
| `ruby` | `**/*.rb` | Rails (routes.rb, controllers); Faraday, Net::HTTP clients |

### Plugin Registration (v1 — Compiled In)

```rust
pub fn default_plugins() -> Vec<Box<dyn LanguagePlugin>> {
    vec![
        // Config plugins (always run)
        Box::new(config::OpenApiPlugin),
        Box::new(config::ProtoPlugin),
        Box::new(config::GraphqlPlugin),
        Box::new(config::AsyncApiPlugin),
        Box::new(config::ComposePlugin),
        Box::new(config::KubernetesPlugin),
        Box::new(config::DockerfilePlugin),
        Box::new(config::EnvPlugin),
        // Language plugins (run when files match)
        Box::new(lang::TypeScriptPlugin),
        Box::new(lang::PythonPlugin),
        Box::new(lang::GoPlugin),
        Box::new(lang::JavaPlugin),
        Box::new(lang::CSharpPlugin),
        Box::new(lang::RustLangPlugin),
        Box::new(lang::RubyPlugin),
    ]
}
```

### Future: External Plugins (v2)

External plugins communicate via stdin/stdout JSON protocol:

```
scanner → plugin stdin:  { "files": [...], "vars": {...} }
plugin  → plugin stdout: { "services": [...], "endpoints": [...], ... }
```

This enables community plugins in any language without recompiling the scanner. The `--plugins` flag already supports filtering, so external plugins can be registered alongside built-in ones.

---

## 6. AST Parsing Strategy

### tree-sitter

All language plugins use **tree-sitter** for AST parsing. tree-sitter is the right choice because:

- **Multi-language with one API.** Same Rust API for TypeScript, Python, Go, Java, C#, Ruby.
- **Fast.** Parses 100K-line files in milliseconds. GitHub uses it for code navigation.
- **Fault-tolerant.** Handles partial/broken files (important for scanning work-in-progress code).
- **Query language.** S-expression queries pattern-match directly against the AST — no manual tree walking.

### Query-Based Extraction

Each framework extractor defines tree-sitter queries. Example for Express.js route detection:

```scheme
;; Express route: app.get("/path", handler)
(call_expression
  function: (member_expression
    object: (identifier) @receiver
    property: (property_identifier) @method)
  arguments: (arguments
    (string) @path
    .
    (_) @handler))
```

The plugin filters results: `@method` must be an HTTP verb (`get`, `post`, `put`, `delete`, `patch`), `@receiver` must match known router variable names.

### Framework Detection Heuristic

Before running AST queries, each language plugin checks for framework markers to avoid wasted parsing:

| Language | Detection Signal |
|---|---|
| TypeScript | `package.json` dependencies: `express`, `@nestjs/core`, `next`, `fastify` |
| Python | `pyproject.toml`/`requirements.txt`: `fastapi`, `django`, `flask` |
| Go | `go.mod`: `github.com/gin-gonic/gin`, `github.com/labstack/echo`, `net/http` (stdlib) |
| Java | `pom.xml`/`build.gradle`: `spring-boot-starter-web` |
| C# | `.csproj`: `Microsoft.AspNetCore` |
| Rust | `Cargo.toml`: `actix-web`, `axum`, `rocket` |
| Ruby | `Gemfile`: `rails`, `sinatra` |

If no framework marker is found, the plugin still runs generic HTTP client detection (fetch, http.Get, etc.) but skips framework-specific route extraction.

---

## 7. Detection Patterns

### 6.1 Service Detection

Services are structural — defined by build config, not code semantics.

| Priority | Signal | Detection | Confidence |
|---|---|---|---|
| 1 | `docker-compose.yml` service block | YAML parse → service names, ports, depends_on | High |
| 2 | `Dockerfile` / `Containerfile` in a directory | Glob match → directory = deployable unit | High |
| 3 | Kubernetes `Deployment` + `Service` manifests | YAML parse → metadata.name, spec.ports | High |
| 4 | `package.json` with `start` script | JSON parse → entry point | High |
| 5 | `pyproject.toml` with entry points / scripts | TOML parse | High |
| 6 | `go.mod` + `main.go` | File existence | High |
| 7 | `Cargo.toml` with `[[bin]]` | TOML parse | High |
| 8 | `.csproj` with `Sdk="Microsoft.NET.Sdk.Web"` | XML parse | High |
| 9 | Monorepo `packages/*/` or `services/*/` | Directory pattern + package manifest existence | Medium |

**Confidence: 90%+.** Merger deduplicates when multiple signals identify the same service (e.g., a `Dockerfile` and a `package.json` in the same directory).

### 6.2 Endpoint Detection

Route definitions are syntactic patterns. Spec files take priority over AST detection.

#### From Spec Files (Config Plugins)

| Source | Extracts | Confidence |
|---|---|---|
| OpenAPI/Swagger | All paths + methods + request/response schemas | High |
| `.proto` files | All `rpc` definitions + request/response message types | High |
| `.graphql` schemas | Queries, mutations, subscriptions + input/output types | High |
| AsyncAPI | Channel definitions + message schemas | High |

#### From Source AST (Language Plugins)

| Framework | Pattern | Confidence |
|---|---|---|
| Express | `app.get("/path", handler)`, `router.post(...)` | High |
| FastAPI | `@app.get("/path")`, `@router.post(...)` | High |
| Django | `urlpatterns = [path("route", view)]` | High |
| Flask | `@app.route("/path", methods=["GET"])` | High |
| Spring Boot | `@GetMapping("/path")`, `@RequestMapping(...)` | High |
| ASP.NET Core | `[HttpGet("path")]`, `[Route("api/[controller]")]` | High |
| Go net/http | `http.HandleFunc("/path", handler)` | High |
| Go Gin | `r.GET("/path", handler)` | High |
| NestJS | `@Get()`, `@Post()` on methods, `@Controller("/path")` on classes | High |
| Rails | `routes.rb`: `get`, `post`, `resources`, `namespace` | High |
| Actix-web | `#[get("/path")]`, `web::resource("/path")` | High |
| Axum | `Router::new().route("/path", get(handler))` | High |

**Deduplication:** When an OpenAPI spec exists alongside source code, the spec is authoritative. Source-detected endpoints are merged with spec endpoints by matching `(method, path)`.

### 6.3 Connection Detection

Connections are the hardest — they're code-level behaviors, not declarations. The scanner detects outbound calls; the hub reconciler matches them to exposed endpoints across repos.

#### HTTP Client Calls

| Language | Library | Pattern | Confidence |
|---|---|---|---|
| TypeScript | fetch | `fetch("/api/users")`, `` fetch(`/api/${path}`) `` | High |
| TypeScript | axios | `axios.get("/api/users")`, `axios({ url: ... })` | High |
| Python | httpx | `httpx.get("http://service/path")` | High |
| Python | requests | `requests.post(url, json=data)` | High |
| Go | net/http | `http.Get("http://service/path")` | High |
| Java | RestTemplate | `restTemplate.getForObject(url, ...)` | High |
| Java | WebClient | `webClient.get().uri("/path")` | High |
| C# | HttpClient | `httpClient.GetAsync("/path")` | High |
| Rust | reqwest | `reqwest::get(url).await` | High |
| Ruby | Faraday | `Faraday.get("/path")` | High |

**Variable resolution:** When the URL argument is a variable (e.g., `fetch(endpoint)`), the scanner traces through:
1. Local assignment in same function/block
2. Module-level constants
3. Import chain (one level)
4. `.env` files (`USER_SERVICE_URL=http://...`)
5. `docker-compose.yml` environment entries
6. Kubernetes ConfigMap data

If resolved → **Medium confidence**. If unresolved → **Low confidence** (still reported with evidence snippet).

#### gRPC Client Calls

| Language | Pattern | Confidence |
|---|---|---|
| Any | `import "service.proto"` in `.proto` files | High |
| Go | `grpc.Dial("service:50051")` | High |
| TypeScript | `new ServiceClient(channel)` matching generated stub names | High |
| Python | `ServiceStub(channel)` | High |
| Java | `ServiceGrpc.newBlockingStub(channel)` | High |
| C# | `new Service.ServiceClient(channel)` | High |

#### Message Queue Calls

| Pattern | Libraries | Confidence |
|---|---|---|
| `channel.publish("topic", data)` | amqplib, pika, lapin | High |
| `channel.subscribe("topic", handler)` | amqplib, pika, lapin | High |
| `producer.send({ topic: "...", ... })` | kafkajs, confluent-kafka, rdkafka | High |
| `consumer.subscribe({ topics: [...] })` | kafkajs, confluent-kafka, rdkafka | High |
| `client.publish("topic", payload)` | mqtt.js, paho-mqtt, rumqttc | High |

**Connection type:** Scanner reports `protocol: "amqp"`, `"kafka"`, or `"mqtt"` and `path` = topic/queue name. Hub reconciler matches publishers to subscribers across repos by topic name.

#### Database Client Calls

| Pattern | Libraries | Protocol |
|---|---|---|
| `pg.connect(connStr)`, `Pool(dsn=...)` | pg, asyncpg, sqlx | `postgresql` |
| `mongoose.connect(uri)` | mongoose, motor | `mongodb` |
| `redis.createClient(url)` | ioredis, redis-py, redis-rs | `redis` |
| `mysql.createConnection(...)` | mysql2, mysqlclient, sqlx | `mysql` |

**Connection target:** Extracted from connection string or env var. If it resolves to a docker-compose service name or k8s Service DNS, the connection targets that service. Otherwise it targets an external database node.

#### Industrial Protocol Calls

Detected through known library call patterns. Protocol stored as free string.

| Protocol | Libraries | Pattern |
|---|---|---|
| MODBUS | pymodbus, libmodbus, tokio-modbus | `ModbusClient(host, port)`, `modbus.connect(...)` |
| OPC UA | opcua, asyncua, open62541 | `Client.connect("opc.tcp://...")` |
| BACnet | BAC0, bacnet-stack | `BAC0.connect(network=...)` |
| CAN bus | python-can, socketcan | `can.Bus(channel=...)` |
| HL7/FHIR | hl7apy, hapi-fhir | `FhirClient(baseUrl)` |

**Confidence: High** when the library is explicitly imported. These are niche but valuable for industrial/healthcare codebases.

### 6.4 Schema Detection

| Source | What It Extracts | Confidence |
|---|---|---|
| OpenAPI spec | Request/response schemas with all field types | High |
| `.proto` messages | Message fields with types | High |
| GraphQL types | Input/output types with fields | High |
| TypeScript interfaces near route handlers | Interface/type fields | High |
| Pydantic BaseModel subclasses | Field names, types, validators | High |
| Go structs with `json:"tag"` | Field names from json tags | High |
| Java DTOs with `@RequestBody` | Field names and types | High |
| C# record/class with `[FromBody]` | Property names and types | High |
| AsyncAPI message schemas | Event payload fields | High |

**Priority rule:** Spec-file schemas override source-code schemas. When OpenAPI defines a response schema for `GET /users`, that takes precedence over the inferred return type in the handler.

---

## 8. Variable Resolution

The variable resolution chain allows the scanner to trace URLs and connection strings from code back to their values.

### Resolution Order (highest priority first)

1. **Inline literal** — `fetch("http://user-service:3000/api/users")` → resolved immediately
2. **Local constant** — `const URL = "http://..."` in same file → follow assignment
3. **Module-level constant** — imported from another file (one level of import tracing)
4. **`.env` files** — `USER_SERVICE_URL=http://user-service:3000`
5. **`docker-compose.yml` environment** — `environment: { USER_SERVICE_URL: "http://..." }`
6. **Kubernetes ConfigMap** — `data: { USER_SERVICE_URL: "http://..." }`
7. **Env var name heuristic** — `process.env.USER_SERVICE_URL` → name implies "user-service" dependency

### VariableStore

**Multiple `.env` file merge order (last wins within each layer):**
`.env` < `.env.local` < `.env.development` < `.env.production`. The scanner reads all `.env*` files and merges them in this order — `.env.local` overrides `.env` for the same key.

```rust
pub struct VariableStore {
    /// Layer 1: merged .env file values (.env < .env.local < .env.development < .env.production)
    env_files: HashMap<String, String>,
    /// Layer 2: docker-compose environment values
    compose_env: HashMap<String, String>,
    /// Layer 3: k8s ConfigMap values
    k8s_env: HashMap<String, String>,
}

impl VariableStore {
    /// Resolve a variable name to its value, checking layers in priority order
    pub fn resolve(&self, key: &str) -> Option<&str>;

    /// Resolve a variable name and extract service target from URL value
    pub fn resolve_to_target(&self, key: &str) -> Option<ServiceTarget>;
}
```

Language plugins call `vars.resolve("USER_SERVICE_URL")` when they encounter an env var access pattern. If the value is a URL, the scanner extracts the hostname as a potential service target.

---

## 9. Intra-Repo Connection Resolution

The scanner's `resolver.rs` handles matching within a single repo. The hub reconciler handles cross-repo matching.

### What the Scanner Resolves

After all plugins run, the merger produces lists of endpoints and outbound calls. The resolver matches them within the same repo:

```
Outbound call:  POST /api/v1/payments  (from order-service)
Exposed endpoint: POST /api/v1/payments (on payment-service)
→ Connection: order-service → payment-service (protocol: rest, method: POST, path: /api/v1/payments)
```

Match criteria: `(method, normalized_path)`.

**Path normalization rules:**

| Input | Normalized | Rule |
|---|---|---|
| `/api/v1/users/:id` | `/api/v1/users/{param}` | Segments starting with `:` → `{param}` |
| `/api/v1/users/{userId}` | `/api/v1/users/{param}` | Segments wrapped in `{}` → `{param}` (name stripped) |
| `/api/v1/users/{id:\\d+}` | `/api/v1/users/{param}` | Regex constraints dropped |
| `/api/v1/files/*` | `/api/v1/files/{*}` | Wildcards → `{*}` |
| `/api/v1/users` | `/api/v1/users` | Static paths unchanged |

**Known limitations:** Regex routes (Express `app.get(/^\/api\//, handler)`) are not normalized and will likely not match. Header-based API versioning (`Accept: application/vnd.api.v2+json`) is not detected. Both are documented as v1 limitations.

### What the Hub Resolves

Outbound calls that don't match any local endpoint are uploaded as connections with `target_name` set to the best guess (hostname from URL, service name from env var, proto service name). The hub's `resolve_dangling_connections()` query matches them when the target repo is uploaded:

```sql
-- Hub matches outbound calls to exposed_endpoints across repos
UPDATE connections SET target_service_id = ep.service_id
FROM exposed_endpoints ep
WHERE connections.target_service_id IS NULL
  AND connections.path = ep.path
  AND connections.method = ep.method
  AND connections.org_id = ep.org_id;
```

---

## 10. Payload Assembly

### Mapping to ScanPayloadV1

The scanner assembles findings into the hub's existing `ScanPayloadV1` format. No hub schema changes needed.

```json
{
  "version": "1.0",
  "metadata": {
    "tool": "cli",
    "tool_version": "0.1.0",
    "scan_mode": "full",              // always "full" in v1
    "repo_url": "git@github.com:acme/order-service.git",
    "repo_name": "order-service",
    "branch": "main",
    "commit_sha": "a1b2c3d4e5f6...",
    "started_at": "2026-04-04T10:00:00Z",
    "completed_at": "2026-04-04T10:00:03Z",
    "files_scanned": 247,
    "project_slug": "acme-platform"
  },
  "findings": {
    "services": [
      {
        "name": "order-service",
        "root_path": ".",
        "language": "typescript",
        "type": "service",
        "boundary_entry": "src/main.ts",
        "confidence": "high",
        "exposes": [
          { "method": "POST", "path": "/api/v1/orders", "handler": "createOrder", "kind": "rest" },
          { "method": "GET", "path": "/api/v1/orders/:id", "handler": "getOrder", "kind": "rest" }
        ]
      }
    ],
    "connections": [
      {
        "source": "order-service",
        "target": "payment-service",
        "protocol": "rest",
        "method": "POST",
        "path": "/api/v1/payments/charge",
        "source_file": "src/services/payment-client.ts:42",
        "confidence": "high",
        "evidence": "await axios.post(`${PAYMENT_URL}/api/v1/payments/charge`, { orderId })"
      },
      {
        "source": "order-service",
        "target": "rabbitmq",
        "protocol": "amqp",
        "method": null,
        "path": "orders.created",
        "source_file": "src/events/publisher.ts:18",
        "confidence": "high",
        "evidence": "channel.publish('orders.created', Buffer.from(JSON.stringify(order)))"
      }
    ],
    "schemas": [
      {
        "name": "CreateOrderRequest",
        "role": "request",
        "file": "src/models/order.ts",
        "connection_ref": null,
        "fields": [
          { "name": "customerId", "type": "string", "required": true },
          { "name": "items", "type": "OrderItem[]", "required": true },
          { "name": "shippingAddress", "type": "Address", "required": false }
        ]
      }
    ],
    "actors": []
  }
}
```

### Field Mapping

| Scanner Type | Payload Field | Notes |
|---|---|---|
| `ServiceInfo` | `findings.services[]` | `extraction_method` not yet in payload — stored in `confidence` text for v1 |
| `EndpointInfo` | `findings.services[].exposes[]` | Nested under the owning service |
| `ConnectionInfo` | `findings.connections[]` | `target_name` maps to `target` |
| `SchemaInfo` | `findings.schemas[]` | Direct mapping |
| `Confidence` | `confidence` string field | "high", "medium", "low" |

### Future Payload Extension

When the hub supports it (v1.1 payload), add:

```json
{
  "extraction_method": "ast:express",  // or "spec:openapi", "config:compose"
  "confidence_score": 0.92             // numeric instead of enum
}
```

This is additive — existing v1.0 payloads still validate. The hub's reconciler uses `extraction_method` to distinguish static-detected vs future LLM-inferred findings.

---

## 11. Upload Protocol

### HTTP Request

```
POST /api/v1/scans/upload
Authorization: Bearer <api-key>
Content-Type: application/json
Content-Length: <bytes>

<ScanPayloadV1 JSON>
```

### Response Codes

| Code | Meaning | Scanner Action |
|---|---|---|
| 202 Accepted | Scan queued for processing | Success — print scan ID |
| 400 Bad Request | Payload validation failed | Print error detail, exit 1 |
| 401 Unauthorized | Invalid or missing API key | Print auth error, exit 1 |
| 409 Conflict | Duplicate commit_sha | Already processed — print message, exit 0 |
| 413 Payload Too Large | Exceeds 10MB limit | Print size error, exit 1 |
| 429 Too Many Requests | Rate limited | Retry with backoff (3 attempts, 1s/2s/4s) |
| 500+ | Server error | Retry with backoff (3 attempts) |

### Retry Policy

- Max 3 retries for 429 and 5xx
- Exponential backoff: 1s, 2s, 4s
- `--output` flag allows saving payload to JSON file for manual upload later

---

## 12. Rust Dependencies

| Crate | Purpose | Version Constraint |
|---|---|---|
| `clap` | CLI argument parsing | 4.x |
| `gix` | Git context (branch, commit, remote) | Latest stable |
| `tree-sitter` | AST parsing engine | 0.24+ |
| `tree-sitter-typescript` | TypeScript/JavaScript grammar | Latest |
| `tree-sitter-python` | Python grammar | Latest |
| `tree-sitter-go` | Go grammar | Latest |
| `tree-sitter-java` | Java grammar | Latest |
| `tree-sitter-c-sharp` | C# grammar | Latest |
| `tree-sitter-rust` | Rust grammar | Latest |
| `tree-sitter-ruby` | Ruby grammar | Latest |
| `reqwest` | HTTP client for upload | 0.12+ (with `rustls-tls`) |
| `serde` + `serde_json` | JSON serialization | 1.x |
| `serde_yaml` | YAML parsing (compose, k8s, openapi) | 0.9+ |
| `toml` | TOML parsing (Cargo.toml, pyproject.toml) | 0.8+ |
| `globset` | Glob pattern matching | 0.4+ |
| `walkdir` | Directory traversal | 2.x |
| `ignore` | .gitignore-aware file walking | 0.4+ |
| `tokio` | Async runtime (for reqwest) | 1.x |
| `anyhow` | Error handling | 1.x |
| `tracing` | Structured logging | 0.1+ |
| `rayon` | Parallel plugin execution | 1.x |

### Build Size Target

Static linking with `musl` target for zero-dependency Linux binary. Expected binary size: ~15-20MB (includes all tree-sitter grammars). Strip symbols for distribution: ~10-12MB.

---

## 13. Monorepo Support

When the scanner detects a monorepo (multiple services in one repo), it reports multiple `ServiceEntry` items in a single payload.

### Detection Signals

1. **docker-compose.yml with multiple services** → one ServiceInfo per service block
2. **Multiple Dockerfiles in different directories** → one ServiceInfo per Dockerfile directory
3. **`packages/*/package.json`** or **`services/*/`** pattern → one ServiceInfo per subdirectory with a package manifest
4. **Multiple `go.mod` files** → one ServiceInfo per module
5. **Multiple `.csproj` files** → one ServiceInfo per project

### File-to-Service Scoping Algorithm

Each source file must be attributed to exactly one service (or no service). The scanner resolves this using **nearest-ancestor matching**:

1. Build a service root map from all detected services: `{ "packages/order-service" → "order-service", "packages/payment-service" → "payment-service" }`
2. For each source file, walk up the directory tree to find the nearest parent that is a service root
3. If found → file belongs to that service
4. If no service root is an ancestor → file is **unscoped** (not attributed to any service)

**Example:**
```
packages/
├── order-service/          ← service root
│   └── src/routes.ts       → belongs to "order-service"
├── payment-service/        ← service root
│   └── src/handler.ts      → belongs to "payment-service"
├── shared/                 ← NOT a service root (no Dockerfile, no start script)
│   └── src/client.ts       → unscoped
└── utils/
    └── logger.ts           → unscoped
```

**Shared libraries** (`packages/shared/`, `libs/`, `common/`): Directories without a service marker (no Dockerfile, no `start` script, no server framework import) are not services. Files in these directories are unscoped. If a language plugin detects an outbound HTTP call in `packages/shared/src/client.ts`, the connection is reported with `source_service = ""` (empty) — the merger drops it with a warning, since a connection without a source service is not actionable.

**Override via `.arcanon.toml`:** If auto-detection misattributes files or misses a service, the `[services]` section in `.arcanon.toml` provides explicit overrides:

```toml
[services."packages/shared"]
ignore = true               # exclude from service detection

[services."packages/gateway"]
name = "api-gateway"        # override auto-detected name
```

---

## 14. Performance Targets

| Metric | Target | How |
|---|---|---|
| Scan time (100 files) | < 2 seconds | tree-sitter parses fast; rayon parallelism |
| Scan time (1,000 files) | < 10 seconds | File I/O dominates; walkdir is efficient |
| Scan time (10,000 files) | < 60 seconds | Glob filtering reduces actual parse count |
| Memory usage | < 200MB peak | Files read on-demand, AST not retained after extraction |
| Binary size | < 15MB | Strip symbols, static link, LTO |
| Upload payload size | < 2MB typical | JSON compression available if needed |

---

## 15. Error Handling

### Scan Errors

Scanning is fault-tolerant. A failure in one file or one plugin does not abort the scan.

| Error | Behavior |
|---|---|
| Single file parse failure | Log warning, skip file, continue |
| Plugin crashes | Catch panic, log error, continue with other plugins |
| No services found | Print warning, still upload (empty findings are valid for cleanup) |
| Git not initialized | Use directory name as repo_name, generate deterministic content hash as commit_sha (SHA-256 of sorted file paths + sizes), warn user |
| .env file missing | Skip, variable resolution falls through to other layers |
| Network unreachable | Save payload to `arcanon-scan-{timestamp}.json`, print path, exit 1 |

### Exit Codes

| Code | Meaning |
|---|---|
| 0 | Success (uploaded or saved) |
| 1 | Upload failed after retries |
| 2 | Invalid arguments |

---

## 16. Future: LLM Enhancement Layer

When LLM is added as a v2 add-on, it becomes an optional post-processing step — either locally or on the hub.

### Option A: Local LLM Enhancement (scanner-side)

```
Static scan → ExtractionResult → [LLM enrichment if enabled] → ScanPayloadV1
```

The LLM receives the static findings plus code context and:
- Resolves ambiguous connection targets
- Detects indirect calls through custom abstractions
- Fuzzy-matches service names ("user-svc" = "user-service")
- Generates human-readable evidence summaries
- Infers schemas from dynamic code

LLM findings are tagged `extraction_method: "llm"` with separate confidence scores.

### Option B: Hub-Side LLM Enhancement

```
Hub receives static payload → Upserts graph → [LLM enrichment on hub] → Updates graph
```

Hub processes stored findings with LLM to find missed connections across repos, using full graph context unavailable to the local scanner.

### Trust Model

Both options tag findings by source. The dashboard can distinguish:
- **Solid lines:** statically detected connections (high confidence)
- **Dashed lines:** LLM-inferred connections (confidence varies)

Users control visibility: "Show only verified connections" toggle hides LLM-inferred edges.

---

## 17. Testing Strategy

### Unit Tests

Each plugin gets its own test suite with fixture files (real code snippets from popular frameworks):

```
tests/
├── fixtures/
│   ├── express-basic/          ← minimal Express app
│   ├── fastapi-basic/          ← minimal FastAPI app
│   ├── spring-boot-basic/      ← minimal Spring Boot app
│   ├── monorepo-compose/       ← docker-compose with 3 services
│   └── ...
├── config/
│   ├── test_openapi.rs
│   ├── test_compose.rs
│   └── ...
├── lang/
│   ├── test_typescript.rs
│   ├── test_python.rs
│   └── ...
├── test_resolver.rs            ← intra-repo connection matching
├── test_merger.rs              ← dedup and merge logic
├── test_payload.rs             ← payload assembly and validation
└── test_vars.rs                ← variable resolution chain
```

### Integration Tests

Scan real open-source repos (pinned commits) and assert expected findings:
- [express-realworld-example-app](https://github.com/gothinkster/node-express-realworld-example-app) → Express routes, MongoDB connection
- [fastapi-realworld-example-app](https://github.com/nsidnev/fastapi-realworld-example-app) → FastAPI routes, PostgreSQL connection
- [microservices-demo](https://github.com/GoogleCloudPlatform/microservices-demo) → Multi-service, gRPC connections, proto files

### Accuracy Metrics

Track per release:
- **Service detection rate:** % of actual services found (target: 90%+)
- **Endpoint detection rate:** % of actual endpoints found (target: 85%+)
- **Connection detection rate:** % of actual connections found (target: 85%+)
- **False positive rate:** % of reported findings that don't exist (target: < 5%)

---

## 18. Distribution

### Binaries

Pre-built for:
- `x86_64-unknown-linux-musl` (Linux amd64, static)
- `aarch64-unknown-linux-musl` (Linux arm64, static)
- `x86_64-apple-darwin` (macOS Intel)
- `aarch64-apple-darwin` (macOS Apple Silicon)
- `x86_64-pc-windows-msvc` (Windows)

### Installation

```bash
# Direct download
curl -fsSL https://get.arcanon.dev/scanner | sh

# Cargo
cargo install arcanon-scanner

# Homebrew (macOS)
brew install arcanon/tap/arcanon-scanner
```

### CI Integration

```yaml
# GitHub Actions
- name: Scan with Arcanon
  run: |
    arcanon-scanner \
      --hub-url ${{ secrets.ARCANON_HUB_URL }} \
      --api-key ${{ secrets.ARCANON_API_KEY }} \
      --project-slug my-project
```

The scanner's `tool` field reports `"cli"` — the hub's `KNOWN_TOOLS` set already includes `"cli"`.
