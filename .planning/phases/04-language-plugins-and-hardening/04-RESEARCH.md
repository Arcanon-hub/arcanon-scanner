# Phase 4: Language Plugins and Hardening - Research

**Researched:** 2026-04-04
**Domain:** tree-sitter AST querying, multi-language framework detection, monorepo scoping
**Confidence:** HIGH (core patterns from architecture doc + official tree-sitter docs; framework-specific query patterns MEDIUM)

---

## Summary

Phase 4 delivers all seven language plugins and hardens monorepo scoping. Each plugin must: (1) check framework markers before committing to full AST parsing (LPLU-08), (2) produce correct endpoint paths including two-phase extraction for prefix-aggregated frameworks (DETQ-05), (3) detect HTTP/gRPC/MQ/DB/industrial protocol client calls (LPLU-09 through LPLU-12), and (4) attribute every finding to its containing service using the nearest-ancestor algorithm (MONO-01 through MONO-03).

The core technical challenge is the two-phase extraction problem: NestJS `@Controller("/prefix")` + `@Get(":id")`, Spring Boot `@RequestMapping("/base")` + `@GetMapping("/sub")`, and ASP.NET Core `[Route("api/[controller]")]` + `[HttpGet("{id}")]` all require building a class-level prefix map in a first pass, then joining it to method-level captures in a second pass. A single query cannot span the class/method node boundary.

The monorepo scoping algorithm is an O(files * depth) nearest-ancestor walk: build a `HashMap<PathBuf, String>` of service root paths, then for each source file walk upward with `path.parent()` until a key is found or the repo root is reached. This is straightforward Rust with `std::path::Path::ancestors()`.

**Primary recommendation:** Implement plugins in dependency order (TypeScript first, then Python, Go, Java, C#, Rust, Ruby), using a shared `AstHelper` in `src/ast/mod.rs` that wraps `QueryCursor::matches()` and provides a `node_text()` / `node_line()` convenience API. Every plugin follows the exact same five-step structure: check framework markers → build parser → define queries → execute queries → return ExtractionResult.

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| LPLU-01 | TypeScript plugin: Express, NestJS, Next.js, Fastify routes + fetch/axios/got clients | Tree-sitter TS grammar covers decorators, call_expression, member_expression. Two-phase for NestJS. |
| LPLU-02 | Python plugin: FastAPI, Django, Flask routes + httpx/requests/aiohttp clients | Decorated_definition node in Python grammar for FastAPI/Flask. Django urlpatterns is a list literal. |
| LPLU-03 | Go plugin: net/http, Gin, Echo, Fiber routes + http.Get/grpc.Dial clients | call_expression with method chain pattern. go.mod marker check. |
| LPLU-04 | Java plugin: Spring Boot @RestController/@RequestMapping + RestTemplate/WebClient | Two-phase for @RequestMapping prefix. annotation node in Java grammar. |
| LPLU-05 | C# plugin: ASP.NET Core [ApiController]/[HttpGet] + HttpClient | Two-phase for [Route] on controller class. attribute node in C# grammar. |
| LPLU-06 | Rust plugin: Actix-web, Axum, Rocket routes + reqwest/tonic clients | macro_invocation for #[get("/path")]. function call chaining for Axum Router::new().route(). |
| LPLU-07 | Ruby plugin: Rails routes.rb, Sinatra + Faraday/Net::HTTP clients | routes.rb requires special handling for resources/namespace expansion. |
| LPLU-08 | Framework marker checks before full AST parsing | File existence checks via ctx.files matching patterns — no extra I/O needed. |
| LPLU-09 | Message queue client detection (amqplib, kafkajs, mqtt.js, pika, rdkafka, rumqttc) | call_expression matching publish/subscribe/send methods with topic capture. |
| LPLU-10 | Database client detection (pg, mongoose, redis, mysql2, sqlx) with protocol ID | call_expression matching connect/createClient patterns + library name → protocol map. |
| LPLU-11 | Industrial protocol detection (pymodbus, opcua, BAC0, python-can, hl7apy) | Library-name-gated call_expression patterns. Low-volume but high-value for ICS codebases. |
| LPLU-12 | gRPC client detection (grpc.Dial, ServiceStub, newBlockingStub, etc.) | Cross-language patterns. Import/require of _pb2_grpc or *_grpc modules is a High-confidence signal. |
| DETQ-05 | Two-phase extraction for NestJS @Controller, Spring @RequestMapping, ASP.NET [Route] | First pass: build class→prefix map. Second pass: join method decorators to class prefix. |
| MONO-01 | Detect monorepos: multiple Dockerfiles, compose services, package manifests, go.mod | Already handled by Phase 3 config plugins + merger. Phase 4 consumes the service root map. |
| MONO-02 | Nearest-ancestor file-to-service attribution | `Path::ancestors()` walk against service root HashMap. O(depth) per file. |
| MONO-03 | Unscoped files (no service ancestor) not attributed to any service | Empty source_service on ConnectionInfo. Merger drops with warning (documented v1 behavior). |
</phase_requirements>

---

## Project Constraints (from CLAUDE.md)

- **Language**: Rust — single binary, no runtime dependencies
- **Binary size**: Target < 15MB stripped (includes all tree-sitter grammars)
- **Performance**: < 2s for 100 files, < 10s for 1,000 files, < 60s for 10,000 files
- **Memory**: < 200MB peak — drop ASTs immediately after extraction, do not retain
- **Dependencies**: Only crates listed in architecture doc section 12
- **Protocol field**: Free `String` — no enum. Use string constants in each plugin.
- **Payload format**: Must match existing hub ScanPayloadV1 exactly
- **Tokio/Rayon boundary**: `extract()` is synchronous (rayon). No `async` in plugin code. No `tokio` imports in `src/plugin/`
- **No regex for code pattern matching** — tree-sitter queries only. Regex causes false positives on nested structures, strings, and comments.

---

## Standard Stack

### Core (already decided in Phase 1)

| Crate | Version | Purpose |
|-------|---------|---------|
| `tree-sitter` | 0.26.8 | Query engine, Parser, QueryCursor |
| `tree-sitter-typescript` | 0.23.2 | TypeScript + JS grammar |
| `tree-sitter-python` | 0.25.0 | Python grammar |
| `tree-sitter-go` | 0.25.0 | Go grammar |
| `tree-sitter-java` | 0.23.5 | Java grammar |
| `tree-sitter-c-sharp` | 0.23.1 | C# grammar |
| `tree-sitter-rust` | 0.24.2 | Rust grammar |
| `tree-sitter-ruby` | 0.23.1 | Ruby grammar |

All grammar crates already in Cargo.toml from Phase 1. No new dependencies for Phase 4.

### No New Dependencies

Phase 4 introduces zero new crates. Every capability needed (AST parsing, file walking, JSON output, logging, parallelism) is already available from Phases 1-3. The plugins are pure Rust logic on top of the existing tree-sitter wrapper in `src/ast/mod.rs`.

---

## Architecture Patterns

### Plugin File Structure

Each of the 7 language plugins lives in `src/plugin/lang/` and follows this structure:

```
src/plugin/lang/
├── mod.rs          ← re-exports all 7 plugins
├── typescript.rs   ← TypeScriptPlugin struct
├── python.rs
├── go.rs
├── java.rs
├── csharp.rs
├── rust_lang.rs
└── ruby.rs
```

### Pattern 1: Standard Plugin Layout (Five Steps)

Every language plugin follows the same five-step structure inside `extract()`:

```
Step 1: Framework Marker Check
  → Search ctx.files for package.json / go.mod / Gemfile
  → If framework not detected: run generic client-only detection, skip route detection
  → If framework detected: proceed to AST parsing

Step 2: Build Parser
  → Parser::new() + parser.set_language(grammar_language())
  → One parser per plugin invocation (not per file) — reset between files

Step 3: Define Queries (constants, compiled once)
  → Route detection queries (per framework variant)
  → Client call queries (HTTP, gRPC, MQ, DB, industrial)
  → Schema queries (optional, per framework)

Step 4: Execute Queries per File
  → For each file in ctx.files: parser.parse(content.as_bytes(), None)
  → QueryCursor::new() + cursor.matches(&query, tree.root_node(), source_bytes)
  → Collect captures into Vec<EndpointInfo> / Vec<ConnectionInfo>

Step 5: Return ExtractionResult
  → Populate services, endpoints, connections, schemas
  → service_name comes from monorepo scoping (see MONO-02 pattern)
```

### Pattern 2: tree-sitter Query Execution (Rust API)

```rust
// Source: docs.rs/tree-sitter + parsiya.net/blog/knee-deep-tree-sitter-queries/

// Compile query once (expensive) — store as a lazy_static or const fn
let query = Query::new(&language, ROUTE_QUERY_SRC)
    .expect("invalid query — compile-time error, not runtime");

// Parse source text
let mut parser = Parser::new();
parser.set_language(&language).expect("grammar load");
let tree = parser.parse(source.as_bytes(), None)
    .ok_or_else(|| anyhow!("parse returned None"))?;

// Execute query — cursor is cheap to create per file
let mut cursor = QueryCursor::new();
let matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

for m in matches {
    for capture in m.captures {
        let node = capture.node;
        let text = node.utf8_text(source.as_bytes())
            .unwrap_or("")
            .trim_matches('"')
            .trim_matches('\'');
        let line = node.start_position().row + 1; // tree-sitter rows are 0-indexed
        let capture_name = query.capture_names()[capture.index as usize];
        // dispatch on capture_name...
    }
}
```

**Key API facts (HIGH confidence — docs.rs/tree-sitter verified 2026-04-04):**
- `node.start_position()` returns `Point { row: usize, column: usize }` (0-indexed rows)
- `node.utf8_text(source_bytes)` returns `Result<&str, Utf8Error>`
- `query.capture_names()` returns `&[&str]` — index matches `capture.index`
- `QueryCursor::matches()` is lazy — iterates on demand, no upfront allocation
- Drop `tree` before next file to avoid accumulating ASTs (ARCHITECTURE anti-pattern 2)

### Pattern 3: Two-Phase Extraction for Prefix-Aggregated Routes

Used by: TypeScript/NestJS, Java/Spring Boot, C#/ASP.NET Core

```
Phase A — Build prefix map (first pass over all files):
  Query: find class-level decorators and capture their path argument
  Output: HashMap<NodeId_or_ClassName, String>  e.g. {"UsersController" → "/users"}

Phase B — Join method decorators to class prefix (second pass):
  Query: find method-level decorators + their enclosing class name
  Join: class_name → lookup in prefix map → prepend to method path
  Output: full endpoint path e.g. "/users/:id"
```

In tree-sitter Rust bindings, "Phase A + B" can be done in a single file pass by:
1. Parsing the file once
2. Walking class declarations to collect prefix decorators into a local `HashMap<&str, &str>`
3. Walking method declarations, looking up their parent class name in the map
4. Building the full path at collection time

This avoids two separate `parser.parse()` calls on the same file.

### Pattern 4: Framework Marker Detection (No Extra I/O)

The plugin receives `ctx.files` which already contains all files matching its declared `file_patterns()`. To check for framework markers, search the already-loaded file list:

```rust
// TypeScript example
fn detect_frameworks(ctx: &ExtractionContext) -> FrameworkSet {
    let mut frameworks = FrameworkSet::default();
    for file in &ctx.files {
        if file.relative_path.ends_with("package.json") {
            let content = &*file.content;
            if content.contains("\"express\"") { frameworks.express = true; }
            if content.contains("\"@nestjs/core\"") { frameworks.nestjs = true; }
            if content.contains("\"next\"") { frameworks.nextjs = true; }
            if content.contains("\"fastify\"") { frameworks.fastify = true; }
        }
    }
    frameworks
}
```

Note: `ctx.files` for the TypeScript plugin includes `**/*.ts`, `**/*.tsx`, `**/*.js`, `**/*.jsx` AND `**/package.json` — the plugin must declare `package.json` in its `file_patterns()` to receive it.

**Alternative approach:** The plugin declares a secondary pattern set for manifest files, separate from source patterns. Process manifest files first (marker check), then process source files only if markers found.

### Pattern 5: Monorepo Nearest-Ancestor Scoping

```rust
// Build once from merged service roots (passed in via ExtractionContext or a helper)
fn scope_file_to_service<'a>(
    file_path: &Path,
    service_roots: &'a HashMap<PathBuf, String>,
) -> Option<&'a str> {
    for ancestor in file_path.ancestors() {
        if let Some(service_name) = service_roots.get(ancestor) {
            return Some(service_name);
        }
    }
    None  // unscoped — shared library or root-level file
}
```

`Path::ancestors()` yields the path itself, then each parent in order up to `/`. This is O(depth) per file and runs entirely in memory — no I/O.

**The service root map** is built by the merger from Phase 3 config plugin outputs. It must be available to language plugins at extraction time. This means either:
- Option A: Pass it via `ExtractionContext` (requires adding a field to the existing struct)
- Option B: Language plugins discover service roots by scanning `ctx.files` for Dockerfile / package.json themselves

Architecture doc section 13 describes this algorithm; `ExtractionContext` should carry the service roots map. This is a Phase 4 addition to `ExtractionContext`.

### Recommended Project Structure Additions (Phase 4)

```
src/plugin/lang/
├── mod.rs
├── typescript.rs       ← LPLU-01
├── python.rs           ← LPLU-02
├── go.rs               ← LPLU-03
├── java.rs             ← LPLU-04
├── csharp.rs           ← LPLU-05
├── rust_lang.rs        ← LPLU-06
├── ruby.rs             ← LPLU-07
tests/
├── fixtures/
│   └── polyglot/       ← end-to-end fixture repo (Phase 4 success criteria #3)
│       ├── service-a/  ← TypeScript/NestJS service with Dockerfile
│       ├── service-b/  ← Python/FastAPI service with Dockerfile
│       └── lib/        ← shared library (unscoped — no Dockerfile)
```

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Language parsing | Custom parsers, regex pattern matching | tree-sitter queries | Regex cannot handle nested structures, strings, or comments. tree-sitter is GitHub-grade, fault-tolerant, parses broken files. |
| AST node type discovery | Manual tree inspection | `tree-sitter-cli playground` or `ts generate` | The playground shows node types interactively for any input snippet. Essential for writing correct queries. |
| File path ancestor iteration | Custom loop | `std::path::Path::ancestors()` | Standard library method. Zero deps, zero allocation. |
| String deduplication | Custom HashSet logic | Standard `HashSet<String>` + merge in merger.rs | Dedup belongs in merger, not in individual plugins. |
| Rails route expansion | AST-walk only | Hardcoded `resources` expansion table | `resources :photos` always generates the same 7 routes. Expansion rules are stable Rails convention, not variable logic. |
| Query compilation | Runtime `Query::new()` per file | `lazy_static!` or `OnceLock` per plugin | `Query::new()` compiles the S-expression. Doing it once per file instead of once per plugin invocation wastes CPU on large repos. |

---

## Framework-Specific Query Patterns

### TypeScript / JavaScript

**Express route detection:**
```scheme
;; app.get("/path", handler) or router.post("/path", handler)
;; Source: architecture.md section 6 + dev.to/lovestaco Express parsing article
(call_expression
  function: (member_expression
    object: (identifier) @receiver
    property: (property_identifier) @method)
  arguments: (arguments
    (string) @path
    (_)* @handler))
```
Filter post-query: `@method` must be in `["get","post","put","delete","patch","head","options","all"]`, `@receiver` must match known router variable names (check against common names like `app`, `router`, `api`, `v1`, `r`).

**NestJS two-phase extraction:**
```scheme
;; Phase A: class-level @Controller("prefix")
(class_declaration
  (decorator
    (call_expression
      function: (identifier) @dec_name
      arguments: (arguments (string) @prefix)))
  name: (type_identifier) @class_name)

;; Phase B: method-level @Get("/:id"), @Post(), etc.
(method_definition
  (decorator
    (call_expression
      function: (identifier) @http_method_dec
      arguments: (arguments (string)? @method_path)))
  name: (property_identifier) @handler_name)
```
Filter: `@dec_name` must be `"Controller"`, `@http_method_dec` must be in `["Get","Post","Put","Delete","Patch"]`. Join Phase A and B by walking the parent class of each method_definition to find its associated `@class_name`.

**TypeScript fetch/axios client detection:**
```scheme
;; fetch(url, options?)
(call_expression
  function: (identifier) @fn
  arguments: (arguments (_) @url (_)* @opts))

;; axios.get(url) / axios.post(url, body)
(call_expression
  function: (member_expression
    object: (identifier) @lib
    property: (property_identifier) @method)
  arguments: (arguments (_) @url (_)*))
```
Filter: `@fn` matches `"fetch"`, `"got"`, `"superagent"`. `@lib` matches `"axios"`, `"got"`, `"superagent"`, `"ky"`. Extract `@url` and resolve via VariableStore if needed.

### Python

**FastAPI/Flask route detection (decorated_definition):**
```scheme
;; @app.get("/path") or @router.post("/path")
;; Source: Python grammar — decorated_definition is the outer node
(decorated_definition
  (decorator
    (call
      function: (attribute
        object: (identifier) @obj
        attribute: (identifier) @http_method)
      arguments: (argument_list (string) @path)))
  definition: (function_definition
    name: (identifier) @handler))
```
Filter: `@http_method` must be in `["get","post","put","delete","patch"]`. `@obj` matches `"app"`, `"router"`, `"api_router"`.

**Django urlpatterns:**
```scheme
;; path("route/", view_function) inside urlpatterns list
(assignment
  left: (identifier) @var_name
  right: (list
    (call
      function: (identifier) @path_fn
      arguments: (argument_list
        (string) @route
        (_) @view))))
```
Filter: `@var_name` must be `"urlpatterns"`, `@path_fn` must be `"path"` or `"re_path"`.

**Python HTTP client detection:**
```scheme
;; requests.get(url) / httpx.post(url) / aiohttp calls
(call
  function: (attribute
    object: (identifier) @lib
    attribute: (identifier) @method)
  arguments: (argument_list (_) @url (_)*))
```
Filter: `@lib` matches `"requests"`, `"httpx"`, `"aiohttp"`, `"urllib"`. `@method` matches HTTP verbs.

### Go

**Gin/Echo/Fiber route detection:**
```scheme
;; r.GET("/path", handler) — Gin, Echo, Fiber all use same pattern
(call_expression
  function: (selector_expression
    operand: (identifier) @router
    field: (field_identifier) @method)
  arguments: (argument_list
    (interpreted_string_literal) @path
    (_)+ @handlers))
```
Filter: `@method` must be in `["GET","POST","PUT","DELETE","PATCH","HEAD","OPTIONS","Any","Handle"]`.

**net/http handler registration:**
```scheme
;; http.HandleFunc("/path", handler) or http.Handle("/path", handler)
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @fn)
  arguments: (argument_list
    (interpreted_string_literal) @path
    (_) @handler))
```
Filter: `@pkg` must be `"http"` or `"mux"`, `@fn` must be `"HandleFunc"` or `"Handle"`.

**Go HTTP client detection:**
```scheme
;; http.Get("url") / http.Post("url", ...) / client.Do(req)
(call_expression
  function: (selector_expression
    operand: (identifier) @obj
    field: (field_identifier) @method)
  arguments: (argument_list (_) @url (_)*))
```
Filter: `@obj` matches `"http"`, `"client"`, `"c"`. `@method` matches `"Get"`, `"Post"`, `"Do"`, `"NewRequest"`.

### Java

**Spring Boot two-phase extraction:**

```scheme
;; Phase A: class-level @RequestMapping("/base") on @RestController class
(class_declaration
  (modifiers
    (annotation
      name: (identifier) @ann_name
      arguments: (annotation_argument_list
        (string_literal) @prefix)?))
  name: (identifier) @class_name)
```
Filter `@ann_name` in `["RestController","Controller","RequestMapping"]`. Collect `@class_name → @prefix` map.

```scheme
;; Phase B: method-level @GetMapping/@PostMapping with path
(method_declaration
  (modifiers
    (annotation
      name: (identifier) @method_ann
      arguments: (annotation_argument_list
        [(string_literal) @path
         (element_value_pair
           key: (identifier) @kv_key
           value: (string_literal) @kv_path)])?))
  name: (identifier) @method_name)
```
Filter `@method_ann` in `["GetMapping","PostMapping","PutMapping","DeleteMapping","PatchMapping","RequestMapping"]`. Join to class prefix from Phase A.

**Note:** Java `@RequestMapping(value="/path", method=RequestMethod.GET)` form uses `element_value_pair` — the query must handle both positional string and named `value=` and `path=` parameters.

### C#

**ASP.NET Core two-phase extraction:**
```scheme
;; Phase A: class-level [Route("api/[controller]")] on controller class
(class_declaration
  (attribute_list
    (attribute
      name: (identifier) @attr_name
      (attribute_argument_clause
        (attribute_argument (string_literal) @route_prefix))))
  name: (identifier) @class_name)
```
Filter `@attr_name` in `["Route","ApiController"]`. Expand `[controller]` token to lowercase class name minus "Controller" suffix.

```scheme
;; Phase B: method-level [HttpGet("{id}")] / [HttpPost]
(method_declaration
  (attribute_list
    (attribute
      name: (identifier) @http_attr
      (attribute_argument_clause
        (attribute_argument (string_literal) @method_path))?))
  name: (identifier) @action_name)
```
Filter `@http_attr` in `["HttpGet","HttpPost","HttpPut","HttpDelete","HttpPatch"]`.

### Rust

**Actix-web macro detection:**
```scheme
;; #[get("/path")] async fn handler(...) { ... }
(attribute_item
  (attribute
    (identifier) @macro_name
    arguments: (token_tree (string_literal) @path)))
(function_item
  name: (identifier) @fn_name)
```
Filter `@macro_name` in `["get","post","put","delete","patch","head","options"]`. The `function_item` immediately follows the `attribute_item` — use the anchor operator `.` to enforce adjacency if needed.

**Axum router detection:**
```scheme
;; Router::new().route("/path", get(handler))
(call_expression
  function: (field_expression
    value: (call_expression) @router_chain
    field: (field_identifier) @method)
  arguments: (arguments
    (string_literal) @path
    (_) @handler))
```
Filter `@method` must be `"route"`. The method+handler is inside the second argument — extract the handler function name from it.

**Rust reqwest/tonic client detection:**
```scheme
;; reqwest::get("url").await or client.get("url")
(call_expression
  function: [
    (scoped_identifier
      path: (identifier) @crate
      name: (identifier) @method)
    (field_expression
      value: (identifier) @client
      field: (field_identifier) @method)
  ]
  arguments: (arguments (_) @url (_)*))
```
Filter `@crate` matches `"reqwest"`, `@method` matches HTTP verbs.

### Ruby

**Rails routes.rb detection:**
```scheme
;; get "/path", to: "controller#action"
;; post "/path", to: "controller#action"
(call
  method: (identifier) @http_verb
  arguments: (argument_list
    (string) @path
    (_)*))
```
Filter `@http_verb` in `["get","post","put","delete","patch","root","match"]`.

**Rails `resources` expansion** — this requires post-query expansion logic (not pure query):
- `resources :users` → expand to 7 standard RESTful endpoints
- `resources :users, only: [:index, :show]` → expand to subset
- `namespace :api { ... }` → prepend `/api/` to all enclosed routes

The query detects `resources` calls; the plugin code expands them using a static lookup table.

**Sinatra route detection (same pattern as Express):**
```scheme
(call
  method: (identifier) @http_verb
  arguments: (argument_list
    (string) @path
    (_)*))
```
Filter `@http_verb` in HTTP verbs. Distinguish from Rails by checking `Gemfile` for `"sinatra"` vs `"rails"`.

---

## Multi-Language Connection Detection Patterns

### HTTP Clients by Language

| Language | Library | Query Target | Evidence Pattern |
|---------|---------|--------------|-----------------|
| TypeScript | fetch | `call_expression` with `identifier` `"fetch"` | `fetch("/api/users")` |
| TypeScript | axios | `call_expression` with `member_expression` object `"axios"` | `axios.get(url)` |
| TypeScript | got | `call_expression` with `member_expression` object `"got"` | `got.get(url)` |
| Python | requests | `call` with `attribute` object `"requests"` | `requests.post(url)` |
| Python | httpx | `call` with `attribute` object `"httpx"` | `httpx.get(url)` |
| Python | aiohttp | `call` with `attribute` object `"session"` after import `"aiohttp"` | `session.get(url)` |
| Go | net/http | `call_expression` with `selector_expression` object `"http"` | `http.Get(url)` |
| Java | RestTemplate | `method_invocation` with object type `"RestTemplate"` | `restTemplate.getForObject(url, ...)` |
| Java | WebClient | `method_invocation` chaining on `"WebClient"` | `webClient.get().uri(path)` |
| C# | HttpClient | `invocation_expression` with `"GetAsync"/"PostAsync"` | `httpClient.GetAsync(url)` |
| Rust | reqwest | `call_expression` with scoped path `reqwest::get` | `reqwest::get(url).await` |
| Ruby | Faraday | `call` on Faraday constant | `Faraday.get("/path")` |
| Ruby | Net::HTTP | `call` on `Net::HTTP` constant | `Net::HTTP.get(uri)` |

### gRPC Clients by Language

| Language | Detection Signal | Confidence |
|---------|-----------------|------------|
| Go | `grpc.Dial("service:port")` + import `"google.golang.org/grpc"` | High |
| TypeScript | `new <ServiceName>Client(channel)` where `<ServiceName>` matches `*_pb` or `_grpc` import | High |
| Python | `<ServiceName>Stub(channel)` where `<ServiceName>Stub` imported from `*_pb2_grpc` module | High |
| Java | `<ServiceName>Grpc.newBlockingStub(channel)` | High |
| C# | `new <ServiceName>.ServiceClient(channel)` | High |
| Rust | `tonic` client builder: `<ServiceName>Client::connect("url")` | High |

**Key insight:** gRPC client detection is most reliable when the import of the generated stub file is detected. If `from order_service_pb2_grpc import OrderServiceStub` is found, then any `OrderServiceStub(channel)` call is High confidence.

### Message Queue Clients

| Library | Language | Publish Pattern | Subscribe Pattern |
|---------|---------|-----------------|-------------------|
| amqplib | TypeScript | `channel.publish(exchange, routingKey, ...)` | `channel.consume(queue, handler)` |
| kafkajs | TypeScript | `producer.send({ topic: "name", ... })` | `consumer.subscribe({ topics: [...] })` |
| mqtt.js | TypeScript | `client.publish("topic", payload)` | `client.subscribe("topic")` |
| pika | Python | `channel.basic_publish(exchange, routing_key, ...)` | `channel.basic_consume(queue, callback)` |
| rdkafka | Rust | `producer.send(FutureRecord::to("topic"))` | `consumer.subscribe(&["topic"])` |
| rumqttc | Rust | `client.publish("topic", qos, ...)` | `client.subscribe("topic", qos)` |
| lapin | Rust | `channel.basic_publish("exchange", "routing_key", ...)` | `channel.basic_consume("queue", ...)` |

**Query pattern (TypeScript/kafkajs example):**
```scheme
;; producer.send({ topic: "topic-name", messages: [...] })
(call_expression
  function: (member_expression
    object: (identifier) @producer
    property: (property_identifier) @method)
  arguments: (arguments
    (object
      (pair
        key: (property_identifier) @key
        value: (string) @topic_name))))
```
Filter: `@method` is `"send"`, `@key` is `"topic"`.

### Database Clients

| Library | Language | Connect Pattern | Protocol |
|---------|---------|-----------------|----------|
| pg / node-postgres | TypeScript | `new Pool(config)` / `pg.connect(connStr)` | `postgresql` |
| mongoose | TypeScript | `mongoose.connect(uri)` | `mongodb` |
| ioredis / redis | TypeScript | `new Redis(url)` / `redis.createClient(url)` | `redis` |
| mysql2 | TypeScript | `mysql.createConnection(config)` | `mysql` |
| asyncpg / psycopg2 | Python | `await asyncpg.connect(dsn)` / `psycopg2.connect(dsn)` | `postgresql` |
| motor | Python | `AsyncIOMotorClient(uri)` | `mongodb` |
| sqlx | Rust | `PgPool::connect(url).await` / `MySqlPool::connect(url)` | `postgresql` / `mysql` |
| redis-rs | Rust | `redis::Client::open(url)` | `redis` |

Connection target extraction: if the connection string is a literal or resolved variable, extract the hostname. If it's a k8s-style DNS name (e.g., `postgres-service.default.svc.cluster.local`), extract the service name.

### Industrial Protocol Clients

| Protocol | Library | Language | Detection Pattern | Confidence |
|---------|---------|---------|------------------|------------|
| Modbus | pymodbus | Python | `ModbusTcpClient(host)` / `ModbusSerialClient(...)` | High (import-gated) |
| Modbus | tokio-modbus | Rust | `tcp::connect(addr)` after `use tokio_modbus::prelude::*` | High |
| OPC UA | opcua | Python | `Client(url="opc.tcp://...")` | High |
| OPC UA | asyncua | Python | `Client(url="opc.tcp://...")` | High |
| BACnet | BAC0 | Python | `BAC0.connect(network=...)` | High |
| CAN | python-can | Python | `can.Bus(channel=..., bustype=...)` | High |
| HL7/FHIR | hl7apy | Python | `Message(name="ADT_A01", ...)` | High |
| HL7/FHIR | hapi-fhir | Java | `FhirContext.forR4()` + `client.create().resource(...)` | High |

**Detection strategy:** Use import/require presence as the primary gate. If `import pymodbus` or `from pymodbus.client import ModbusTcpClient` appears in the file, then any matching call expression is High confidence. Without the import, it would be Low confidence (name collision risk).

---

## Monorepo Support Details

### MONO-01: Detection Signals (Phase 3 config plugins → Phase 4 consumes)

The monorepo detection is entirely driven by config plugin outputs from Phase 3:
- `DockerfilePlugin` → one ServiceInfo per Dockerfile directory
- `ComposePlugin` → one ServiceInfo per compose service block
- Multiple `package.json` with start scripts → one ServiceInfo each
- Multiple `go.mod` files → one ServiceInfo each

By the time Phase 4 language plugins run, the service root map is already populated by the merger. Phase 4 only needs to consume it.

### MONO-02: Nearest-Ancestor Algorithm

```rust
// Build map once from merger output (before language plugins run)
let service_roots: HashMap<PathBuf, String> = merged_services
    .iter()
    .map(|s| (repo_root.join(&s.root_path), s.name.clone()))
    .collect();

// Per-file scoping (called inside each plugin's extract())
fn scope_to_service<'a>(
    file_abs_path: &Path,
    service_roots: &'a HashMap<PathBuf, String>,
) -> Option<&'a str> {
    file_abs_path
        .ancestors()
        .find_map(|ancestor| service_roots.get(ancestor))
        .map(String::as_str)
}
```

This uses `Iterator::find_map` which stops at the first match — O(depth) worst case, O(1) best case (file is directly inside a service root).

### MONO-03: Unscoped Files

When `scope_to_service()` returns `None`, the file's findings are still recorded but with `source_service = String::new()`. The merger identifies these and emits a `tracing::warn!()` before dropping connections that have no source service. Endpoints without a service are also dropped (cannot be attributed in the payload).

This behavior is documented as a v1 limitation. The `--verbose` output must clearly list which files were unscoped and what findings were dropped.

---

## Schema Extraction Patterns

Schema extraction from source code is lower priority than route/connection detection but required for completeness:

| Source | Tree-sitter Target | What to Extract |
|--------|-------------------|-----------------|
| TypeScript `interface` near handler | `interface_declaration` → `property_signature` nodes | field name, type, optional marker |
| TypeScript `type` alias with object type | `type_alias_declaration` → `object_type` | field name, type |
| Python Pydantic `BaseModel` subclass | `class_definition` with base `BaseModel` → `expression_statement` assignments | field name, type annotation |
| Go struct with json tags | `type_declaration` → `struct_type` → `field_declaration_list` | field name from `tag_literal` |
| Java `@RequestBody` class | `class_declaration` with `@RequestBody` parameter in adjacent method → field names | field names, types |
| C# `record` or `class` with `[FromBody]` | similar attribute-gated approach | property names, types |

**Priority rule from architecture doc:** Spec-file schemas (OpenAPI, proto, GraphQL) override source-code schemas when both exist. Source schema extraction is Medium confidence. Only emit source schemas for endpoints that have no spec-file schema.

---

## Common Pitfalls

### Pitfall 1: Two-Phase Extraction Forgotten (DETQ-05)
**What goes wrong:** Query detects `@Get("/:id")` but not the `@Controller("/users")` prefix on the class → endpoint reported as `GET /:id` instead of `GET /users/:id`. All NestJS/Spring/ASP.NET endpoints become incorrect.

**Prevention:** Write fixture tests for prefix-aggregated patterns before declaring those plugins complete. The fixture must have at minimum: class with prefix decorator + method with path decorator → verify full path in ExtractionResult.

**Detection:** Unit test with `@Controller("/users") + @Get("/:id")` fixture produces `GET /users/:id` in endpoint list.

### Pitfall 2: Query Returns Empty on Valid Code
**What goes wrong:** The S-expression query is structurally correct but doesn't match because the actual node type names in the grammar differ from what was assumed. Example: Python function calls use `call` not `call_expression`; Go uses `interpreted_string_literal` not `string`.

**Prevention:** Use `tree-sitter-cli` playground (`npm install -g tree-sitter-cli && tree-sitter playground`) or the online playground at `https://tree-sitter.github.io/tree-sitter/playground` to inspect actual node types for each language before writing queries. Always verify against real input snippets.

**Detection:** Query returns 0 matches on a fixture file that clearly contains the target pattern. Add a `tracing::debug!("query produced {} matches", match_count)` log inside each plugin.

### Pitfall 3: Grammar Node Types Differ from Intuition
**What goes wrong:** Different grammars use different names for similar constructs:
- Python: `call` (not `call_expression`), `attribute` (not `member_expression`)
- Go: `call_expression` (same as TS), `interpreted_string_literal` (not `string`)
- Ruby: `call` with `method:` field (not `function:`)
- Java: `method_invocation` (not `call_expression`)
- C#: `invocation_expression` (not `call_expression`)

**Prevention:** Every plugin must be developed against the correct grammar. Use `ts generate` + `node-types.json` from each grammar crate's source to confirm node type names.

**Confidence:** HIGH — verified via tree-sitter grammar source files and established blog examples.

### Pitfall 4: Rails `resources` Routes Missed
**What goes wrong:** `resources :users` appears in routes.rb but the plugin only detects literal `get "/path"` calls. All resourceful routes (7 routes per resource) produce zero endpoints.

**Prevention:** After querying for literal route methods, query separately for `resources` and `namespace` calls. Implement a static expansion table:
```
resources :photos → [GET /photos, GET /photos/new, POST /photos,
                      GET /photos/:id, GET /photos/:id/edit,
                      PUT /photos/:id, DELETE /photos/:id]
```
Flag all expanded routes at Medium confidence (Rails may apply `only:` / `except:` options that can't be statically resolved).

### Pitfall 5: framework_markers Check Doesn't Include Manifest in file_patterns
**What goes wrong:** `TypeScriptPlugin::file_patterns()` only returns `["**/*.ts", "**/*.tsx", "**/*.js", "**/*.jsx"]`. The plugin never receives `package.json` files so framework detection always fails, and NestJS/Express routes are never detected.

**Prevention:** Each language plugin that does marker-based detection must declare the manifest files in `file_patterns()`. The plugin then partitions `ctx.files` into manifest files (for marker detection) and source files (for AST parsing).

### Pitfall 6: QueryCursor Reuse Across Files Without Reset
**What goes wrong:** `QueryCursor` can carry state. If the same cursor is reused across multiple files without calling `cursor.set_point_range()` or creating a new cursor, results from a previous file may bleed into the next.

**Prevention:** Create a new `QueryCursor::new()` per file, or call the reset methods between files. The cost of creating a `QueryCursor` is negligible compared to parse time.

### Pitfall 7: `[controller]` Token Not Expanded in ASP.NET Routes
**What goes wrong:** ASP.NET `[Route("api/[controller]")]` uses `[controller]` as a template token. The scanner captures the literal string `"api/[controller]"` instead of expanding it to `"api/users"` (from `UsersController`).

**Prevention:** After capturing the route template, check for `[controller]` token and replace it with the lowercase class name minus "Controller" suffix. Example: `UsersController` → `users`.

---

## Anti-Patterns to Avoid

- **Regex for AST extraction:** Never use `content.contains("@Get")` or `Regex::new(r"@Get\(\"(.*?)\"\)")`. tree-sitter queries handle string escaping, multiline decorators, and comments correctly; regex does not.
- **Single-pass for two-phase frameworks:** Attempting to capture class prefix and method path in one query pattern will fail — the AST has class decorators and method decorators at different tree depths with no direct path between them.
- **Retaining parsed trees after extraction:** `let tree = parser.parse(...)` must be dropped before the next file. Storing trees in a `Vec` across all files of a large repo will exhaust memory.
- **Hard-coding service names in plugin code:** Plugins must use the monorepo scoping algorithm to determine `source_service`. Hard-coding `"my-service"` breaks any monorepo.
- **Treating `None` path arguments as empty string:** Some decorators have no path argument (e.g., NestJS `@Get()` with no args defaults to `""`). Distinguish between "no path" (inherits controller prefix only) and "empty path" — both produce different results.

---

## Implementation Order

Implement plugins in this order for maximum early value:

1. **TypeScript** (LPLU-01) — Highest prevalence, most complex (NestJS two-phase), validates the two-phase pattern
2. **Python** (LPLU-02) — FastAPI/Django/Flask, validates the decorated_definition pattern
3. **Go** (LPLU-03) — Gin/Echo, validates the chained method call pattern
4. **Java** (LPLU-04) — Spring Boot, second two-phase example
5. **C#** (LPLU-05) — ASP.NET Core, third two-phase example, validates `[controller]` expansion
6. **Rust** (LPLU-06) — Actix/Axum, validates macro_invocation and router chain patterns
7. **Ruby** (LPLU-07) — Rails, validates resources expansion logic

Each plugin should be delivered with its own unit test fixture before moving to the next.

### Polyglot Fixture Repo

Phase 4 success criterion #3 requires an end-to-end fixture repo:
```
tests/fixtures/polyglot/
├── service-a/                    ← TypeScript/NestJS service
│   ├── Dockerfile
│   ├── package.json              ← contains "@nestjs/core"
│   └── src/
│       ├── users.controller.ts   ← @Controller("/users") + @Get("/:id") → GET /users/:id
│       └── http-client.ts        ← axios.post(PAYMENT_URL) → connection to payment-service
├── service-b/                    ← Python/FastAPI service
│   ├── Dockerfile
│   ├── requirements.txt          ← contains "fastapi"
│   └── main.py                   ← @app.get("/items/{id}") → GET /items/{id}
└── lib/                          ← shared library, no Dockerfile
    └── shared_client.py          ← httpx.get(SERVICE_URL) → connection with null source_service
```

Scanner run against `tests/fixtures/polyglot/` must produce:
- service-a scoped to TypeScript files under `service-a/`
- service-b scoped to Python files under `service-b/`
- `lib/shared_client.py` is unscoped (no service ancestor with Dockerfile)
- Connection from service-a to payment-service via axios.post
- Endpoint `GET /users/:id` under service-a (two-phase extraction verified)

---

## Environment Availability

| Dependency | Required By | Available | Version | Fallback |
|------------|-------------|-----------|---------|----------|
| Rust toolchain | All compilation | Yes | 1.93.1 (stable) | — |
| cargo | Build system | Yes | 1.93.1 | — |
| rustfmt | `make fmt` | Yes | (bundled with stable) | — |
| clippy | `make lint` | Yes | (bundled with stable) | — |
| x86_64-unknown-linux-musl target | musl binary CI build | No | — | Add via `rustup target add x86_64-unknown-linux-musl` in CI |
| tree-sitter CLI | Node type discovery during development | No (not required at runtime) | — | Use online playground at tree-sitter.github.io |

**Note on musl target:** The current machine has only `aarch64-apple-darwin` installed. The musl target is required for CI (Phase 1 BLDG-06) but not for local development of Phase 4 language plugins. No action needed for Phase 4 specifically.

---

## State of the Art

| Old Approach | Current Approach | Impact |
|--------------|-----------------|--------|
| Regex-based route detection | tree-sitter S-expression queries | Handles multiline, nested, commented-out code correctly |
| Single-pass route extraction | Two-phase extraction for prefix-aggregated frameworks | Correct full paths for NestJS/Spring/ASP.NET |
| Hard-coded service name | Nearest-ancestor monorepo scoping | Works correctly in any monorepo topology |
| Per-file `Query::new()` | `OnceLock<Query>` or `lazy_static!` per plugin | 10-100x faster on large repos (avoids recompiling S-expressions) |

---

## Open Questions

1. **ExtractionContext service roots field**
   - What we know: `ExtractionContext` is defined in Phase 1. Monorepo scoping needs the service root map inside `extract()`.
   - What's unclear: Was `service_roots: HashMap<PathBuf, String>` added to `ExtractionContext` in Phase 1/2/3, or does Phase 4 need to add it?
   - Recommendation: Phase 4 Wave 0 must check `ExtractionContext` definition and add the field if missing. This requires a change to `src/plugin/mod.rs` and `src/types/mod.rs` — it's not a plugin-only change.

2. **Query compilation strategy**
   - What we know: `Query::new()` is expensive (S-expression compilation). Should be done once per plugin, not once per file.
   - What's unclear: Whether to use `lazy_static!`, `std::sync::OnceLock`, or a plugin `new()` constructor that stores compiled queries.
   - Recommendation: Use `std::sync::OnceLock<Query>` (stable since Rust 1.70) stored as a static in each plugin module. This avoids the `lazy_static` crate dependency.

3. **Rails `namespace` nesting depth**
   - What we know: `namespace :api { namespace :v1 { resources :users } }` produces `/api/v1/users` prefix.
   - What's unclear: How deep do real Rails apps nest namespaces? Is 3-level nesting common enough to warrant recursive namespace expansion?
   - Recommendation: Implement 2-level namespace depth as a v1 target; document deeper nesting as a v1 limitation.

4. **Axum router chain depth**
   - What we know: Axum routes are registered as `Router::new().route("/path", get(handler)).route("/other", post(handler2))`.
   - What's unclear: The tree-sitter query for chained `.route()` calls — does the query need to handle arbitrarily deep method chains?
   - Recommendation: Use a query that matches individual `.route(path, method_router)` calls without caring about the chain context. Each `.route()` call is an independent endpoint declaration.

---

## Sources

### Primary (HIGH confidence)
- `docs/architecture.md` (this project) — Section 6 (AST parsing strategy), Section 7 (detection patterns), Section 13 (monorepo support)
- `.planning/research/STACK.md` — Tree-sitter version compatibility table
- `.planning/research/PITFALLS.md` — Pitfalls 3, 6, 12 directly relevant to Phase 4
- [tree-sitter Rust API — docs.rs/tree-sitter](https://docs.rs/tree-sitter/latest/tree_sitter/) — Node, Query, QueryCursor API
- [tree-sitter Query Operators — official docs](https://tree-sitter.github.io/tree-sitter/using-parsers/queries/2-operators.html) — anchors, alternations, optional, wildcards
- [tree-sitter Predicates — official docs](https://tree-sitter.github.io/tree-sitter/using-parsers/queries/3-predicates-and-directives.html) — #eq?, #match?, #any-of?

### Secondary (MEDIUM confidence)
- [Knee Deep in tree-sitter Queries — parsiya.net](https://parsiya.net/blog/knee-deep-tree-sitter-queries/) — Rust query execution patterns (QueryCursor::matches, node text extraction)
- [Express.js AST Parsing with tree-sitter — dev.to/lovestaco](https://dev.to/lovestaco/getting-started-with-tree-sitter-syntax-trees-and-express-api-parsing-5c2d) — Express route query S-expression
- [Spring Boot tree-sitter extraction — medium.com/@linz07m](https://medium.com/@linz07m/extracting-endpoints-and-handlers-from-spring-boot-java-code-using-tree-sitter-73c3481e1b69) — Java annotation query pattern

### Tertiary (LOW confidence — needs validation against actual grammar)
- Query patterns for Python, Go, C#, Rust, Ruby derived from grammar source analysis + architecture doc patterns. All must be validated against `tree-sitter playground` before use.

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — no new dependencies, all from Phase 1
- tree-sitter query API: HIGH — verified from docs.rs
- Framework-specific query S-expressions: MEDIUM — patterns based on grammar structure analysis + community examples; each must be validated against tree-sitter playground before implementation
- Monorepo algorithm: HIGH — standard `Path::ancestors()` traversal, fully specified in architecture doc
- Industrial protocol patterns: MEDIUM — library names verified, call patterns derived from library docs

**Research date:** 2026-04-04
**Valid until:** 2026-10-04 (tree-sitter grammar APIs are stable; framework-specific patterns may evolve if grammars are updated)
