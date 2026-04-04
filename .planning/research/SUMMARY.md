# Project Research Summary

**Project:** arcanon-scanner
**Domain:** Rust CLI multi-language static code analyzer / service dependency scanner
**Researched:** 2026-04-04
**Confidence:** HIGH

## Executive Summary

Arcanon Scanner is a single-binary Rust CLI that performs static AST analysis across 7 programming languages and 8 configuration formats to extract service topology (services, endpoints, connections, schemas) and upload a structured payload to the Arcanon Hub. The product occupies a narrow intersection between static analysis tooling (Semgrep, OWASP Noir) and service catalog feeders (Backstage), doing neither job but combining insights from both. The recommended approach is a pure Rust implementation with tree-sitter for AST parsing, rayon for CPU-bound parallelism, and tokio/reqwest for async upload. The architecture is a strict pipeline: discover files, run plugins in parallel, merge results, resolve intra-repo connections, assemble payload, upload. No step reads from a later step — information flows forward only.

The stack is fully specified and verified against crates.io as of 2026-04-04. Key non-obvious choices are `gix` over `git2` (pure Rust, no C dependency, enables static musl binary), `serde_yaml_bw` over `serde_yaml` (the latter is archived and deprecated), and rayon over tokio for plugin parallelism (tree-sitter parsing is CPU-bound, not I/O-bound). The architecture document in `docs/architecture.md` is the authoritative source for component boundaries and data flow; no architectural speculation is needed.

The principal risks are build-time (tree-sitter grammar version conflicts that prevent compilation, and binary size bloat from compiled C grammar code) and logic-level (the service merger producing duplicate services from overlapping detection signals, and the variable resolution chain producing wrong connection targets in polyglot `.env` environments). The Tokio/rayon deadlock risk is a runtime trap that manifests only when async code is introduced inside plugin `extract()` calls — a hard sync/async boundary must be enforced from day one. All critical pitfalls have clear mitigations documented in PITFALLS.md; none require architectural changes if caught early.

---

## Key Findings

### Recommended Stack

The stack is 100% Rust with no C library dependencies (except tree-sitter grammars, which compile to native code via `build.rs`). The minimum supported Rust version is 1.85, imposed by `clap` 4.6.0. The static binary target is `x86_64-unknown-linux-musl`; expected stripped binary size is 10–15MB with proper LTO settings (`lto = "fat"`, `codegen-units = 1`, `strip = "symbols"`).

The two highest-risk dependency choices require explicit attention: (1) grammar crates pin tree-sitter core versions independently — all 7 grammar crates must be locked to a consistent core ABI version using Cargo's dependency resolution, verified via `cargo tree --duplicates | grep tree-sitter` in CI; (2) `serde_yaml` is deprecated and must not be used — `serde_yaml_bw` 2.5.4 is the drop-in replacement with active maintenance and identical API.

**Core technologies:**
- `clap` 4.6.0: CLI argument parsing with derive macros and env var support — de facto standard, eliminates boilerplate
- `tree-sitter` 0.26.8 + 7 grammar crates: Multi-language AST parsing — only production-grade multi-language option with pre-built grammars
- `ignore` 0.4.25: Gitignore-aware file walking — the ripgrep engine, strictly superior to `walkdir`
- `rayon` 1.11.0: CPU-bound parallel plugin execution — work-stealing parallelism, one-line parallel iterator conversion
- `reqwest` 0.13.2 with `rustls-tls`: Async HTTP upload — no OpenSSL dependency, required for static binary distribution
- `tokio` 1.51.0 (minimal features): Async runtime for upload only — `rt-multi-thread` + `macros` only; no tokio file I/O
- `gix` 0.81.0: Pure-Rust git context detection — no libgit2 C dependency, static musl-safe
- `serde` + `serde_json` + `toml` + `serde_yaml_bw`: Serialization stack — covers JSON, TOML, and YAML config formats
- `anyhow` 1.0.102: Application-level error handling — correct choice for binary crates (not `thiserror`)
- `tracing` + `tracing-subscriber`: Structured logging — integrates with tokio, supports `RUST_LOG` and `-v/-vv/-vvv`

### Expected Features

Arcanon Scanner has a well-defined hub contract and is greenfield — all v1 features are required for the product to function, not just to feel complete. The key table stakes features that cannot be deferred are: single binary distribution, zero-config first run, structured JSON output (ScanPayloadV1), meaningful exit codes (0/1/2), monorepo support with nearest-ancestor scoping, fault-tolerant scanning (never abort on a single file error), and git context attachment to every scan upload for idempotency.

**Must have (table stakes):**
- Single binary, no runtime install — prerequisite for CI adoption
- `.gitignore` respect — scanners that traverse `node_modules` are abandoned immediately
- `--dry-run` and `--output` flags — required for developer trust and script integration
- Meaningful exit codes (0 = success, 1 = fatal, 2 = upload failed) — CI pipelines gate on these
- Git context (branch, commit SHA, repo URL) — enables idempotent hub uploads and scan history
- Monorepo support with service scoping — majority of multi-service codebases are monorepos
- Fault-tolerant per-file error handling — single malformed file must not abort the scan
- 3x exponential backoff on upload — network reliability in CI
- Environment variable auth (`ARCANON_API_KEY`, `ARCANON_HUB_URL`) — CI secret management standard
- Human-readable summary to stdout + JSON payload — dual-audience output

**Should have (differentiators):**
- Variable resolution chain (`.env` → docker-compose `env:` → k8s ConfigMaps) — dereferences `${DB_HOST}` to actual target names; unique to topology tools
- Confidence-tagged findings (High/Medium/Low) — enables hub-side quality filtering; raw scanners don't do this
- Evidence snippets on connections with `file:line` attribution — enables hub-to-code deep links
- Intra-repo connection resolution (outbound call matched to local endpoint within same repo) — pre-resolved edges reduce hub reconciliation work
- Config-driven service name overrides and manual connection declarations — enterprise adoption enabler
- Industrial protocol detection (Modbus, OPC UA, BACnet) — differentiates from security-only scanners
- Framework detection heuristic before AST parsing — performance and correctness optimization
- 8 config format plugins (OpenAPI, proto, GraphQL, AsyncAPI, docker-compose, Kubernetes, Dockerfile, .env) — structural completeness
- 7 language plugins via tree-sitter — TypeScript, Python, Go, Java, C#, Rust, Ruby

**Defer (v2+):**
- External plugin protocol (stdin/stdout JSON) — designed, not built; 15 compiled-in plugins are sufficient for v1
- LLM enhancement layer — non-deterministic, cloud dependency, deferred optionality
- Incremental scanning / file caching — full scans are < 10s at v1 scale; ROI insufficient
- Numeric confidence scores (0.0–1.0) — High/Medium/Low enum covers v1 needs; v1.1 extension
- IDE plugin, Homebrew tap, macOS/Windows CI — distribution infrastructure for post-validation

### Architecture Approach

The architecture is a strict forward-only pipeline defined in `docs/architecture.md`: Configuration → Context Gathering (git + variable store, parallel) → File Discovery → Plugin Execution (rayon parallel) → Merge → Intra-Repo Resolution → Payload Assembly → Upload/Save. The `LanguagePlugin` trait is the single interface between all plugin code and the core engine; plugins are stateless structs that receive an `ExtractionContext` and return an `ExtractionResult`. The core engine never changes when a new plugin is added. File content is shared via `Arc<str>` to avoid copies; ASTs are local to `extract()` scope and dropped immediately after query execution to meet the 200MB memory target.

**Major components:**
1. `types/mod.rs` — Shared types (FileContext, ExtractionResult, ServiceInfo, EndpointInfo, ConnectionInfo, SchemaInfo, Confidence); no runtime logic; must exist before any other module
2. `plugin/mod.rs` — LanguagePlugin trait + compiled-in registry (`default_plugins()`); defines the boundary between core and all plugins
3. `ast/mod.rs` — tree-sitter wrapper; query execution and node traversal helpers; used by all language plugins
4. `vars/mod.rs` — VariableStore built from `.env` + docker-compose + k8s ConfigMaps; read-only after construction; passed as `Arc<VariableStore>` to all plugins
5. `git/mod.rs` — GitContext detection via gix with CI env var fallbacks; called once at startup
6. Config plugins (`src/plugin/config/*.rs`) — 8 plugins parsing spec/infra files; no AST dependency; simpler to implement first
7. Language plugins (`src/plugin/lang/*.rs`) — 7 plugins using tree-sitter queries for framework detection and AST extraction
8. `core/merger.rs` — deduplication by service name (normalized) and root_path; only component that sees all plugin outputs simultaneously
9. `core/resolver.rs` — intra-repo outbound-to-endpoint matching by `(method, normalized_path)`
10. `core/payload.rs` — maps internal types to ScanPayloadV1 JSON schema
11. `upload/mod.rs` — HTTP POST with auth, 3x exponential backoff, error handling; only async code outside `main.rs`
12. `main.rs` + `core/scanner.rs` — CLI entry point and step sequencer

### Critical Pitfalls

1. **tree-sitter grammar/core version mismatch** — Multiple grammar crates pulling in different tree-sitter core versions causes a compile error ("found a different tree_sitter::Language"). Pin all grammar crates deliberately; add `cargo tree --duplicates | grep tree-sitter` to CI. Must be resolved in Phase 1 before writing any plugin code.

2. **Tokio/rayon deadlock** — Calling `.await` or `block_on` from inside plugin `extract()` (which runs on rayon threads) hangs the scanner silently. Enforce a hard boundary: plugins are synchronous, tokio is only used in the upload module. No `tokio` imports allowed in `src/plugin/`. Detect via: scanner hangs after "Scanning complete" with CPU at 0%.

3. **Service merger creating duplicate services** — Multiple plugins independently detect the same service with different names (e.g., `"order-service"` vs `"order_service"` vs `"api"`). The merger must normalize names AND merge by `root_path` proximity, not name equality alone. Establish signal priority: compose key > package.json `name` > Dockerfile directory > language plugin inference.

4. **Prefix-aggregated route patterns breaking endpoint detection** — NestJS `@Controller("/users") + @Get("/:id")`, Spring Boot `@RequestMapping + @GetMapping`, ASP.NET `[Route] + [HttpGet]` split route paths across class and method AST nodes. A single query misses the prefix. Requires two-phase extraction: build class-prefix map first, then join method decorators to it. Affects TypeScript, Java, and C# plugins.

5. **serde_yaml is deprecated** — The `serde_yaml` crate was archived in March 2024. Use `serde_yaml_bw` 2.5.4 instead (drop-in replacement). Add `cargo audit` to CI to catch this and future deprecations.

---

## Implications for Roadmap

Based on the architecture's component dependency graph and the feature priority analysis, the natural build order is: foundation types + infrastructure → config plugins (simpler, no AST) → pipeline core (merger/resolver/payload/upload) → language plugins (complex, incremental) → full integration and hardening.

### Phase 1: Foundation and Build Infrastructure

**Rationale:** Everything else depends on `types/mod.rs` and the `LanguagePlugin` trait. CI build infrastructure (musl target, LTO profile, binary size assertion, `cargo tree --duplicates` check) must be set up before grammar crates multiply the risk of version conflicts. The `serde_yaml` choice must be made before any YAML parsing code is written.
**Delivers:** Compiling project skeleton with `types/mod.rs`, `plugin/mod.rs` trait, `ast/mod.rs` tree-sitter wrapper, `git/mod.rs`, `vars/mod.rs` stub, Cargo.toml with all dependencies pinned and verified, CI pipeline (clippy, fmt, test, musl build, binary size assertion, `cargo tree --duplicates` check)
**Addresses:** Single binary distribution, Rust 1.85 MSRV, musl static target
**Avoids:** Pitfall 1 (grammar version mismatch), Pitfall 2 (serde_yaml), Pitfall 10 (binary size)

### Phase 2: File Discovery, Git Context, and Variable Resolution

**Rationale:** The `ignore` crate, `gix` integration, and `VariableStore` must be in place before any plugin can run. Variable resolution is a shared service that config plugins and language plugins both depend on. Getting CI env var fallbacks right (Pitfall 11) and `.env` multi-file conflict handling right (Pitfall 7) is easier to isolate and test before plugins complicate the picture.
**Delivers:** Working `core/scanner.rs` step 1–3 (config load, context gather, file discovery), `VariableStore` built from `.env` + docker-compose + k8s, `GitContext` with CI env var fallbacks, built-in excludes, integration test for detached HEAD scenario
**Addresses:** gitignore respect, env var auth, git context, monorepo file scoping
**Avoids:** Pitfall 7 (wrong connection targets from variable resolution), Pitfall 11 (detached HEAD)

### Phase 3: Config Plugins and Pipeline Core

**Rationale:** Config plugins (OpenAPI, proto, docker-compose, Dockerfile, Kubernetes, .env, GraphQL, AsyncAPI) have no AST dependency — they parse YAML, JSON, and structured text files. They are faster to implement and produce high-confidence results that validate the merger and payload assembly logic before language plugins add complexity. The merger, resolver, payload, and upload modules form the complete pipeline that every language plugin will flow through. Wiring this end-to-end with one simple plugin (Dockerfile) proves the full pipeline before AST work begins.
**Delivers:** All 8 config plugins, `core/merger.rs` with name normalization and root_path deduplication, `core/resolver.rs` intra-repo path matching, `core/payload.rs` ScanPayloadV1 assembly, `upload/mod.rs` with retry, `--dry-run`/`--output`/exit codes, human-readable stdout summary, end-to-end scan working with config plugins only
**Addresses:** OpenAPI/proto/GraphQL/AsyncAPI endpoints, docker-compose/k8s service detection, variable resolution from compose env and ConfigMaps, idempotent upload, fault-tolerant scanning
**Avoids:** Pitfall 5 (duplicate services from overlapping signals), Pitfall 8 (YAML anchors), Pitfall 13 (unscoped connection drops), Pitfall 14 (payload size)

### Phase 4: Language Plugins — Tier 1 (TypeScript, Python, Go)

**Rationale:** These three languages cover the majority of modern microservice codebases. TypeScript/NestJS has the most complex framework patterns (prefix-aggregated routes, Pitfall 3) and must be implemented correctly to prove out the two-phase extraction approach that Java and C# will also require. Implementing TypeScript first maximizes learning before approaching Spring Boot and ASP.NET.
**Delivers:** TypeScript plugin (Express, NestJS, Fastify — two-phase route extraction), Python plugin (FastAPI, Django, Flask), Go plugin (net/http, Chi, Gin, Echo), connection detection patterns for all three, schema extraction from typed models, full test fixtures for prefix-aggregated patterns
**Addresses:** Table stakes multi-language coverage, confidence tagging, evidence snippets, source file + line attribution
**Avoids:** Pitfall 3 (prefix-aggregated routes), Pitfall 6 (tree-sitter query explosion on large TS files)

### Phase 5: Language Plugins — Tier 2 (Java, C#, Rust, Ruby)

**Rationale:** These languages expand coverage but have lower prevalence in modern cloud-native stacks than Tier 1. Java/Spring Boot and C#/ASP.NET share the prefix-aggregated route pattern with NestJS — the pattern is proven by Phase 4. Ruby/Rails resourceful routes require special expansion logic (`resources :users` generates 7 routes) that is scoped entirely to the Ruby plugin.
**Delivers:** Java plugin (Spring Boot, Micronaut — two-phase extraction), C# plugin (ASP.NET Core — two-phase extraction), Rust plugin (Actix, Axum), Ruby plugin (Rails with `resources` expansion), full test coverage for each
**Addresses:** Enterprise language coverage, Rails resourceful routes
**Avoids:** Pitfall 3 (prefix-aggregated routes — Java, C#), Pitfall 12 (Rails resourceful routes)

### Phase 6: Hardening, Integration Testing, and CI Documentation

**Rationale:** With all plugins implemented, end-to-end integration tests against real-world repo fixtures validate the merger's multi-signal deduplication, the resolver's path normalization, and the payload's correctness. Performance validation against 1,000-file monorepos confirms the < 10s CI time budget. CI documentation (GitHub Actions snippet, GitLab CI snippet) enables adoption.
**Delivers:** Integration test suite against real polyglot fixture repos, monorepo scoping validation, performance benchmark (< 10s / 1,000 files, < 200MB peak memory), CI documentation (GitHub Actions, GitLab CI), `.arcanon.toml` service override and manual connection validation, `cargo audit` integrated in CI
**Addresses:** Fault tolerance validation, monorepo scoping, performance within CI budgets, CI pipeline integration documentation
**Avoids:** Pitfall 9 (fixture self-scan), all previously documented pitfalls validated end-to-end

### Phase Ordering Rationale

- `types/mod.rs` must precede every other module — it defines the data structures all components share
- The `LanguagePlugin` trait boundary must be finalized before any plugin is written — later plugins require no core engine changes
- Config plugins before language plugins: simpler (no AST), validates the full pipeline earlier, faster time to first end-to-end scan
- Tier 1 language plugins before Tier 2: higher adoption, complex patterns (NestJS prefix routing) proven before similar patterns in Java and C#
- Merger tested with multi-signal fixtures in Phase 3 before language plugins add a third category of detection signals
- The Tokio/rayon boundary is enforced structurally from Phase 1: the upload module is async, plugins are sync, never mixed

### Research Flags

Phases likely needing deeper research during planning:
- **Phase 3 (Config Plugins):** YAML anchor and multi-document handling in `serde_yaml_bw` — test with Helm-generated manifests and complex docker-compose anchors before committing to the parser approach. The YAML ecosystem post-`serde_yaml` deprecation has fragmented; `serde_yaml_bw` is the pragmatic choice but should be validated against real fixture files in early Phase 3.
- **Phase 4 (TypeScript Plugin):** NestJS two-phase route extraction from AST — requires research into tree-sitter's parent-node traversal to build the class-prefix map. Query performance on large generated `.d.ts` and `.pb.ts` files needs profiling with real monorepo samples.
- **Phase 5 (Java Plugin):** Spring Boot's `@RequestMapping` inheritance hierarchy — Spring allows route prefixes on superclasses. Scope this limitation explicitly before implementation to avoid open-ended scope creep.

Phases with well-documented patterns (can skip research-phase):
- **Phase 1 (Foundation):** Cargo.toml dependency setup and musl build config are fully specified in STACK.md with verified crate versions
- **Phase 2 (File Discovery + Git):** `ignore` crate and `gix` integration are well-documented; CI env var fallback chain is fully specified in ARCHITECTURE.md
- **Phase 6 (Hardening):** Integration test patterns for CLI tools are standard; no domain-specific research needed

---

## Confidence Assessment

| Area | Confidence | Notes |
|------|------------|-------|
| Stack | HIGH | All versions verified against crates.io 2026-04-04; grammar ABI compatibility confirmed via tree-sitter's backwards-compatibility policy; one MEDIUM item: `serde_yaml_bw` is the pragmatic choice but YAML ecosystem is fragmented post-deprecation |
| Features | HIGH | Table stakes derived from OWASP Noir, Semgrep, Trivy, CLIG.dev — well-established patterns; differentiators validated against architecture doc's hub contract |
| Architecture | HIGH | Primary source is `docs/architecture.md` — project-specific authoritative document with Rust type signatures; build order derived from component dependency graph with no ambiguity |
| Pitfalls | HIGH | Critical pitfalls confirmed via official GitHub issue trackers; grammar version conflict (Pitfall 1), serde_yaml deprecation (Pitfall 2), tree-sitter query performance (Pitfall 6), Tokio/rayon deadlock (Pitfall 4) all sourced from official trackers or Rust Users Forum |

**Overall confidence:** HIGH

### Gaps to Address

- **YAML parser validation:** `serde_yaml_bw` handles YAML anchors, merge keys, and multi-document files — this should be validated with Helm-generated Kubernetes manifests early in Phase 3, not assumed. The YAML ecosystem is fragmented and `serde_yaml_bw` is the pragmatic choice but has not been battle-tested at scale.
- **tree-sitter grammar ABI compatibility at compile time:** Grammar crates at 0.23.x are documented as compatible with tree-sitter 0.26.x core via backwards-compatibility policy, but were not individually compiled against 0.26.8 as part of research. The first `cargo build` in Phase 1 will confirm or reveal conflicts.
- **Variable resolution chain edge cases:** The 7-step resolution chain is specified in the architecture doc; tracing imports across module boundaries (e.g., Python `os.environ` accessed through a `config.py` module) is the hardest step and likely to produce edge cases in Phases 4–5. These are expected v1 limitations, not blockers — document them as known limitations in verbose output.
- **Intra-repo resolver path normalization:** Path normalization rules for matching `/:id` → `/{id}` across frameworks (Express colon syntax vs OpenAPI brace syntax) need fixture-driven validation in Phase 3. Edge cases with regex route patterns and optional path segments are scoped to v2.

---

## Sources

### Primary (HIGH confidence)
- `docs/architecture.md` — authoritative project architecture document; component boundaries, data flow, type signatures
- crates.io API — all crate versions verified 2026-04-04
- tree-sitter GitHub issue #3095 — grammar/core version conflict confirmed
- tree-sitter GitHub issue #973 — query performance confirmed
- serde_yaml docs.rs "0.9.34+deprecated" — deprecation confirmed
- Rust Users Forum (serde_yaml deprecation thread) — community alternatives assessed
- NestJS official docs (docs.nestjs.com/controllers) — prefix-aggregated route pattern confirmed
- Docker Compose GitHub issue #10824 — YAML anchor bug confirmed
- Tokio official guidance (Rust Users Forum sync lock in async) — deadlock mechanics confirmed

### Secondary (MEDIUM confidence)
- tree-sitter packaging blog (ayats.org) — ABI conflict corroboration
- Knee Deep in tree-sitter Queries (parsiya.net) — query complexity practitioner experience
- Mixing rayon and tokio (Lobsters) — deadlock community experience
- Datadog static analyzer migration blog — rayon + tree-sitter production usage pattern
- Semgrep multicore monorepo blog — performance expectations for parallel scanning
- serde-saphyr crate (0.0.23, 3K downloads) — evaluated and rejected for YAML parsing

### Tertiary (LOW confidence)
- DevOpsSchool top static analysis tools survey — table stakes feature survey
- Virima service dependency mapping tools — service map feature expectations
- Arnica incremental SCA scanning strategies — monorepo incremental scan patterns
- Jellyfish platform engineering anti-patterns — developer friction patterns

---
*Research completed: 2026-04-04*
*Ready for roadmap: yes*
