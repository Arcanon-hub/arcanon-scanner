# Architecture Patterns

**Project:** Arcanon Scanner
**Researched:** 2026-04-04
**Primary Source:** `docs/architecture.md` (authoritative design document, HIGH confidence)

---

## Recommended Architecture

The architecture is fully specified in `docs/architecture.md`. This document extracts component
boundaries, data flow, and build-order implications for roadmap planning.

### System Overview

```
Source Code Repo (disk)
        │
        ▼ (ignore crate — .gitignore-aware walk)
┌─────────────────────────────────────────────────────────────────┐
│  arcanon-scanner (single binary)                                │
│                                                                 │
│  ┌──────────┐   ┌──────────────────────────────────────────┐   │
│  │ CLI      │   │ Plugin Host                               │   │
│  │ (clap)   │──▶│                                           │   │
│  └──────────┘   │  Config Plugins (always run, parallel):  │   │
│                 │    openapi, proto, graphql, asyncapi,     │   │
│  ┌──────────┐   │    compose, kubernetes, dockerfile, env  │   │
│  │ Git      │   │                                           │   │
│  │ Context  │   │  Language Plugins (on match, parallel):  │   │
│  │ (gix)    │   │    typescript, python, go, java,         │   │
│  └──────────┘   │    csharp, rust_lang, ruby               │   │
│                 └──────────────┬─────────────────────────┘   │
│  ┌──────────┐                  │ Vec<ExtractionResult>        │
│  │ Variable │                  ▼                              │
│  │ Store    │   ┌──────────────────────────────────────────┐   │
│  │ (.env,   │   │ Core Engine                               │   │
│  │  compose,│   │   merger.rs  → dedup, merge all results  │   │
│  │  k8s)   │──▶│   resolver.rs → intra-repo call matching  │   │
│  └──────────┘   │   payload.rs  → assemble ScanPayloadV1   │   │
│                 └──────────────┬─────────────────────────┘   │
│                                │ ScanPayloadV1 JSON            │
│                 ┌──────────────▼─────────────────────────┐   │
│                 │ Upload (reqwest + rustls-tls)            │   │
│                 │   POST /api/v1/scans/upload             │   │
│                 │   retry: 3x exponential backoff         │   │
│                 └─────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
        │
        ▼
  Arcanon Hub (external — already exists)
```

---

## Component Boundaries

| Component | Location | Responsibility | Communicates With |
|-----------|----------|----------------|-------------------|
| `main.rs` | `src/main.rs` | CLI entry point: parse args, orchestrate all steps, print summary | All components (top-level orchestrator) |
| `core/scanner.rs` | `src/core/scanner.rs` | Step sequencing: discover → extract → merge → resolve → payload | All other core modules, plugin host |
| `git/mod.rs` | `src/git/mod.rs` | Detect repo_url, branch, commit_sha via `gix` with CI env var fallbacks | Called once at startup by scanner.rs |
| `vars/mod.rs` | `src/vars/mod.rs` | Build `VariableStore` from `.env`, docker-compose env, k8s ConfigMaps | Read by scanner.rs; passed to plugins via `ExtractionContext` |
| File Discovery | `src/core/scanner.rs` (inline) | Walk tree with `ignore` crate, apply guards (size, binary, line length) | Feeds file lists to plugin host |
| `plugin/mod.rs` | `src/plugin/mod.rs` | `LanguagePlugin` trait + compiled-in registry (`default_plugins()`) | Called by scanner.rs; provides interface to all plugins |
| Config Plugins | `src/plugin/config/*.rs` | Parse spec/infra files (OpenAPI, proto, GraphQL, AsyncAPI, compose, k8s, Dockerfile, .env) | Receive `ExtractionContext`, return `ExtractionResult` |
| Language Plugins | `src/plugin/lang/*.rs` | Detect frameworks, parse source AST with tree-sitter queries, detect connections | Receive `ExtractionContext` + `VariableStore`, return `ExtractionResult` |
| `ast/mod.rs` | `src/ast/mod.rs` | tree-sitter wrapper: query execution, node traversal helpers | Used by all language plugins (and some config plugins) |
| `core/merger.rs` | `src/core/merger.rs` | Deduplicate services by name, merge endpoint lists, aggregate connections across all plugin outputs | Takes `Vec<ExtractionResult>`, returns merged set |
| `core/resolver.rs` | `src/core/resolver.rs` | Match outbound calls to local endpoints within same repo by `(method, normalized_path)` | Takes merged results from merger.rs, returns resolved connections |
| `core/payload.rs` | `src/core/payload.rs` | Map internal types to `ScanPayloadV1` JSON matching hub's exact schema | Takes resolved results + git context, returns serialized JSON |
| `upload/mod.rs` | `src/upload/mod.rs` | HTTP POST to hub with auth, retry (3x exponential backoff), error codes | Called last by scanner.rs; optionally bypassed with `--output`/`--dry-run` |
| `types/mod.rs` | `src/types/mod.rs` | Shared types: `FileContext`, `ExtractionResult`, `ServiceInfo`, `EndpointInfo`, `ConnectionInfo`, `SchemaInfo`, `Confidence` | Imported by all other modules; no runtime logic |

---

## Data Flow

Information moves strictly forward through the pipeline. There is no back-channel between phases.

```
Step 1: Configuration
  CLI args + .arcanon.toml
       │
       ▼
  Merged ScanConfig (precedence: CLI > env > toml > defaults)

Step 2: Context Gathering (parallel)
  gix ──────────────────────────▶ GitContext { repo_url, branch, commit_sha }
  .env + compose + k8s ─────────▶ VariableStore { env_files, compose_env, k8s_env }

Step 3: File Discovery
  ignore::WalkBuilder(root)
    + built-in excludes
    + .gitignore respect
    + .arcanon.toml excludes
    + size/binary/line-length guards
       │
       ▼
  Vec<FileContext> { path, relative_path, content: Arc<str> }

Step 4: Plugin Execution (rayon parallel)
  For each plugin:
    ExtractionContext { files: matching_files, vars: Arc<VariableStore>, root }
       │
       ▼ plugin.extract()
    ExtractionResult { services, endpoints, connections, schemas, actors }
       │
  Vec<ExtractionResult>  (one per plugin)

Step 5: Merge
  Vec<ExtractionResult>
       │ merger.rs: dedup services by name, merge endpoint lists
       ▼
  MergedResult { services: Vec<ServiceInfo>, endpoints: Vec<EndpointInfo>,
                 connections: Vec<ConnectionInfo>, schemas: Vec<SchemaInfo> }

Step 6: Intra-Repo Resolution
  MergedResult
       │ resolver.rs: match outbound calls → local endpoints by (method, normalized_path)
       ▼
  ResolvedResult (connections now have source+target services where resolvable)

Step 7: Payload Assembly
  ResolvedResult + GitContext + ScanConfig
       │ payload.rs
       ▼
  ScanPayloadV1 JSON

Step 8: Upload or Save
  ScanPayloadV1
       │ upload/mod.rs (--output → file, --dry-run → stdout, default → POST /api/v1/scans/upload)
       ▼
  Arcanon Hub (202 Accepted) or local JSON file
```

**Key isolation points:**
- Plugins are completely isolated from each other — they do not read each other's output
- `VariableStore` is read-only after construction (built before plugins run)
- `FileContext.content` is `Arc<str>` — file contents are read once and shared without copying
- The merger is the only component that sees all plugin outputs simultaneously
- The resolver sees only the merged output, never raw plugin results

---

## Patterns to Follow

### Pattern 1: LanguagePlugin Trait (Compiled-In Registry)

**What:** All plugins implement a single `LanguagePlugin` trait. The registry is a `Vec<Box<dyn LanguagePlugin>>` returned by `default_plugins()`. No dynamic loading, no external processes.

**When:** Used for every extraction step. The trait boundary is the only interface between plugin code and the core engine.

**Build implication:** The trait must be finalized before any plugin is implemented. Adding a language plugin in a later phase requires no changes to the core engine.

```rust
pub trait LanguagePlugin: Send + Sync {
    fn name(&self) -> &str;
    fn file_patterns(&self) -> &[&str];
    fn always_run(&self) -> bool { false }
    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult;
}
```

### Pattern 2: ExtractionResult as Universal Output Type

**What:** Every plugin returns the same `ExtractionResult` struct regardless of whether it's a config plugin or a language plugin. The merger handles all results uniformly.

**When:** This simplifies the merger — it doesn't need to know which plugin produced which result.

**Build implication:** `ExtractionResult` and all its field types must be defined in `types/mod.rs` before any plugin is written.

### Pattern 3: Config-Before-Language Execution Order

**What:** Config plugins (OpenAPI, proto, etc.) run alongside language plugins in parallel (via rayon), but their results take priority in deduplication. When a spec file and AST analysis both detect the same endpoint, the spec file's version is canonical.

**When:** Any time the same endpoint or schema appears in both a spec file and source code.

### Pattern 4: Variable Resolution as a Shared Service

**What:** `VariableStore` is built once before plugins run and passed as `Arc<VariableStore>` to all plugins. No plugin triggers its own `.env` read.

**When:** Any plugin that encounters an env var reference (`process.env.X`, `os.getenv("X")`) calls `vars.resolve("X")` on the shared store.

### Pattern 5: Fault-Tolerant Scanning

**What:** Every plugin error is caught (including panics), logged as a warning, and skipped. A scan that encounters 50 file-level errors still produces and uploads its complete findings for the files that succeeded.

**When:** Parsing is inherently risky (malformed code, unusual syntax). Never abort a scan for individual file failures.

---

## Anti-Patterns to Avoid

### Anti-Pattern 1: Cross-Plugin Dependencies

**What:** Plugin A trying to read Plugin B's output during the extraction phase.

**Why bad:** Plugins run in parallel. Ordering them would eliminate parallelism and create a dependency graph that grows exponentially as plugins are added.

**Instead:** All outputs are merged in `merger.rs` after all plugins complete. If Plugin B's findings are needed to inform Plugin A's behavior, that cross-referencing belongs in `merger.rs` or `resolver.rs`.

### Anti-Pattern 2: Retaining ASTs After Extraction

**What:** Storing a parsed tree-sitter AST in memory after `extract()` returns.

**Why bad:** Memory target is 200MB peak. A single 50K-line TypeScript file produces a large AST. Retaining ASTs for 1,000 files would blow the memory budget.

**Instead:** Parse the AST, run queries, collect string results, drop the AST. `FileContext.content` (the source text) is shared via `Arc<str>` but AST nodes are local to the `extract()` call.

### Anti-Pattern 3: Hard-Coding Protocol as Enum

**What:** Representing the `protocol` field on `ConnectionInfo` as a Rust enum.

**Why bad:** Industrial protocols (Modbus, OPC UA, BACnet, CAN) are first-class citizens. An enum requires a code change to add every new protocol and breaks forward-compatibility with future hub payload versions.

**Instead:** `protocol` is a free `String`. Use constants in each plugin (`const PROTOCOL_REST: &str = "rest"`) if desired, but the wire format is always a string.

### Anti-Pattern 4: Global Mutable State in Plugins

**What:** Plugins that accumulate state across invocations or share state across threads.

**Why bad:** `LanguagePlugin` is `Send + Sync`. If plugins hold mutable state, parallel execution via rayon requires locks, eliminating performance gains and risking deadlocks.

**Instead:** Plugins are stateless structs. All mutable state lives in the local `extract()` function scope. The `ExtractionResult` is the only output.

### Anti-Pattern 5: Scanning Files Outside the Plugin's Declared Patterns

**What:** A plugin walking the entire file tree itself inside `extract()` rather than using the `ctx.files` it receives.

**Why bad:** Bypasses the built-in exclude rules, binary guards, size limits, and .gitignore rules. Creates duplicated I/O work.

**Instead:** Declare all needed file patterns in `file_patterns()`. The core engine routes the right files to each plugin.

---

## Build Order Implications

The component dependency graph determines which phases must complete before others start.

```
Layer 0 (must exist first — everything depends on these):
  types/mod.rs
    FileContext, ExtractionResult, ServiceInfo, EndpointInfo,
    ConnectionInfo, SchemaInfo, ActorInfo, Confidence, VariableStore

Layer 1 (infrastructure — used by plugins):
  ast/mod.rs          ← tree-sitter wrapper
  vars/mod.rs         ← variable resolution chain
  git/mod.rs          ← git context detection
  plugin/mod.rs       ← LanguagePlugin trait + ExtractionContext

Layer 2 (config plugins — no AST dependency, simpler to implement):
  plugin/config/dockerfile.rs   ← simplest: glob match = service
  plugin/config/env.rs          ← feeds VariableStore, needed by Layer 3
  plugin/config/compose.rs      ← services, connections, env vars
  plugin/config/kubernetes.rs   ← services, ConfigMaps, env vars
  plugin/config/openapi.rs      ← endpoints, schemas (YAML/JSON parse)
  plugin/config/proto.rs        ← gRPC services, message schemas
  plugin/config/graphql.rs      ← queries, mutations, types
  plugin/config/asyncapi.rs     ← channels, event schemas

Layer 3 (core pipeline — requires Layer 0 + Layer 1 complete):
  core/merger.rs      ← dedup + merge (stateless, pure function)
  core/resolver.rs    ← path normalization + intra-repo matching
  core/payload.rs     ← ScanPayloadV1 assembly + JSON serialization
  upload/mod.rs       ← HTTP client with retry

Layer 4 (language plugins — requires Layer 0 + Layer 1; can be added incrementally):
  Each language plugin is independent. Suggested order by bang-for-buck:
    1. typescript.rs  ← highest prevalence in modern microservices
    2. python.rs      ← FastAPI/Django/Flask wide usage
    3. go.rs          ← common in cloud-native
    4. java.rs        ← Spring Boot enterprise
    5. csharp.rs      ← ASP.NET Core
    6. rust_lang.rs   ← Actix/Axum (eats own dogfood)
    7. ruby.rs        ← Rails (lowest modern prevalence)

Layer 5 (CLI entry point — requires all other layers):
  main.rs             ← clap parsing, top-level orchestration
  core/scanner.rs     ← step sequencer (calls all other components)
```

**Critical path for a working end-to-end scan:**

```
types → plugin trait → [one config plugin OR one language plugin] → merger → resolver → payload → upload → main.rs
```

A minimal working scanner can be achieved with as few as: types + plugin trait + dockerfile plugin (for service detection) + merger + resolver + payload + upload + main.rs. Every additional plugin incrementally improves coverage.

---

## Scalability Considerations

| Concern | At 100 files | At 1,000 files | At 10,000 files |
|---------|--------------|----------------|-----------------|
| File I/O | Trivial (<0.5s) | Walkdir dominates (~3-5s) | Glob filtering critical to avoid parsing 10K files |
| AST parsing | Fast (<1s total) | rayon parallelism keeps this <5s | tree-sitter's C core handles this; grammar compile-time matters more than parse-time |
| Memory | Well under 200MB | Files are `Arc<str>` — shared safely | Drop ASTs immediately; don't accumulate all file contents simultaneously |
| Payload size | <100KB | ~500KB | May approach 2MB limit; hub accepts up to 10MB |
| Plugin count | No concern (15 plugins, parallel) | No concern | No concern; plugin count is fixed at compile time |

**Performance design decisions already embedded:**
- `rayon` for parallel plugin execution (Section 12 of architecture doc)
- `Arc<str>` for file content sharing without copies
- Glob filtering eliminates non-matching files before any parsing
- tree-sitter is built in C with Rust bindings — parse speed is not a bottleneck
- ASTs are local to `extract()` scope — GC'd immediately after query runs

---

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Component boundaries | HIGH | Defined precisely in `docs/architecture.md` Sections 4-5 with Rust type signatures |
| Data flow | HIGH | Execution flow diagram in Section 4 is unambiguous; type signatures enforce the flow |
| Build order | HIGH | Derived from the component dependency graph; no external research needed |
| tree-sitter integration | HIGH | Well-established library; architecture doc includes example queries (Section 6) |
| Plugin parallelism via rayon | HIGH | Standard pattern; rayon is the documented choice |
| Merger/resolver logic | MEDIUM | Logic is clearly specified; edge cases (monorepo scoping, path normalization) may surface implementation surprises |
| Variable resolution completeness | MEDIUM | The 7-step resolution chain is defined; tracing imports across module boundaries is the hardest step and likely to have edge cases |

---

## Sources

- `docs/architecture.md` — Primary source; project-specific, HIGH confidence, written 2026-04-04
- `src/main.rs` file structure in architecture doc Section 4 (project structure tree)
- Plugin trait and type definitions from architecture doc Section 5
- Execution flow from architecture doc Section 4 (12-step numbered list)
- Performance targets from architecture doc Section 14
- Dependency list from architecture doc Section 12
