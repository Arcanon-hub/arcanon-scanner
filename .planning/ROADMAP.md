# Roadmap: Arcanon Scanner

## Overview

Arcanon Scanner is built in four phases that follow the natural dependency graph of the codebase. Phase 1 establishes the shared types, plugin trait, tree-sitter wrapper, CLI skeleton, and build tooling that every subsequent phase depends on. Phase 2 adds the runtime infrastructure — file walking, git context, and variable resolution — that all plugins require as inputs. Phase 3 completes the full pipeline end-to-end using only config plugins (no AST), proving the merger, resolver, payload, and upload logic before language plugins add complexity. Phase 4 delivers all seven language plugins and hardens monorepo scoping, producing the complete v1 scanner.

## Phases

**Phase Numbering:**
- Integer phases (1, 2, 3): Planned milestone work
- Decimal phases (2.1, 2.2): Urgent insertions (marked with INSERTED)

Decimal phases appear between their surrounding integers in numeric order.

- [ ] **Phase 1: Foundation** - Shared types, plugin trait, tree-sitter wrapper, CLI skeleton, build tooling, and CI
- [ ] **Phase 2: Infrastructure** - File discovery, git context detection, and variable resolution
- [ ] **Phase 3: Pipeline and Config Plugins** - All 8 config plugins wired through merger, resolver, payload assembly, and upload — end-to-end scan working
- [ ] **Phase 4: Language Plugins and Hardening** - All 7 language plugins with AST extraction, monorepo scoping, and fault tolerance validation

## Phase Details

### Phase 1: Foundation
**Goal**: A compiling project skeleton with all shared types, the plugin trait boundary, tree-sitter wrapper, CLI argument parsing, and a green CI pipeline
**Depends on**: Nothing (first phase)
**Requirements**: CLI-01, CLI-02, CLI-03, CLI-04, CLI-05, CLI-06, CLI-07, CLI-08, CLI-09, CLI-10, CLI-11, BLDG-01, BLDG-02, BLDG-03, BLDG-04, BLDG-05, BLDG-06, BLDG-07
**Success Criteria** (what must be TRUE):
  1. `cargo build --release` produces a single binary under 15MB with the musl target
  2. `arcanon-scanner --help` and `arcanon-scanner --version` work with correct output
  3. All CLI flags (--hub-url, --api-key, --project-slug, --output, --dry-run, --plugins, --exclude, -v/-vv/-vvv, --repo-url, --branch, --commit-sha) are parsed without error
  4. `make lint`, `make fmt`, `make test`, `make build` all succeed
  5. CI passes on push: clippy with denied warnings, rustfmt check, cargo test, musl binary build, and `cargo tree --duplicates | grep tree-sitter` returns clean
**Plans**: 4 plans

Plans:
- [ ] 01-01-PLAN.md — Cargo.toml with pinned dependencies and all source module stubs
- [ ] 01-02-PLAN.md — All shared types, LanguagePlugin trait, AstParser wrapper, VariableStore stub
- [ ] 01-03-PLAN.md — Full CLI entry point (clap Cli struct, tracing init, argument parsing tests)
- [ ] 01-04-PLAN.md — Makefile targets and GitHub Actions CI workflow

### Phase 2: Infrastructure
**Goal**: The scanner can discover all eligible files in a repo, attach verified git context, and build a populated VariableStore before any plugin runs
**Depends on**: Phase 1
**Requirements**: DISC-01, DISC-02, DISC-03, DISC-04, DISC-05, GIT-01, GIT-02, GIT-03, GIT-04, VARS-01, VARS-02, VARS-03, VARS-04, VARS-05
**Success Criteria** (what must be TRUE):
  1. Scanner walks a repo with nested .gitignore files and excludes node_modules/, target/, vendor/ and other built-in paths without traversing symlinks
  2. Running against a git repo produces correct repo_url, branch, and commit SHA in the output; running in CI with GITHUB_REF_NAME set uses that value as the branch
  3. Running with --repo-url, --branch, --commit-sha overrides all auto-detected git values
  4. A repo containing .env, docker-compose.yml, and a Kubernetes ConfigMap produces a VariableStore where `${DB_HOST}` resolves to its actual value from the appropriate source
**Plans**: 3 plans

Plans:
- [ ] 02-01-PLAN.md — File discovery module: walk_repo() with built-in excludes, binary guard, line-length guard, symlink skip
- [ ] 02-02-PLAN.md — Git context module: detect_git_context() with gix + full CI env-var fallback chain
- [ ] 02-03-PLAN.md — Variable resolution module: VariableStore from .env, docker-compose, k8s ConfigMap sources

### Phase 3: Pipeline and Config Plugins
**Goal**: A complete end-to-end scan using config plugins only — discovers files, runs all 8 config plugins in parallel, merges results, resolves intra-repo connections, assembles a valid ScanPayloadV1, and uploads it or writes it to a file
**Depends on**: Phase 2
**Requirements**: CPLU-01, CPLU-02, CPLU-03, CPLU-04, CPLU-05, CPLU-06, CPLU-07, CPLU-08, PIPE-01, PIPE-02, PIPE-03, PIPE-04, PIPE-05, UPLD-01, UPLD-02, UPLD-03, UPLD-04, FTOL-01, FTOL-02, FTOL-03, FTOL-04, DETQ-01, DETQ-02, DETQ-03, DETQ-04, MONO-04
**Success Criteria** (what must be TRUE):
  1. Running against a repo with an OpenAPI spec, a Dockerfile, and a docker-compose.yml produces a ScanPayloadV1 JSON that passes hub validation (hub returns 202)
  2. `--dry-run` prints the payload to stdout with exit 0 and makes no HTTP request; `--output result.json` writes valid JSON to that file without uploading
  3. A corrupted YAML file causes a logged warning and the scan continues; a plugin panic is caught and other plugins complete normally
  4. The merger produces one service entry for a service detected by both the Dockerfile plugin and the compose plugin (no duplicates), with connections aggregated from both
  5. The upload module retries on a 429 response and succeeds on the second attempt; a 409 (duplicate) response exits 0
**Plans**: 5 plans

Plans:
- [ ] 03-01-PLAN.md — Infrastructure config plugins: DockerfilePlugin, EnvPlugin, ComposePlugin, KubernetesPlugin
- [ ] 03-02-PLAN.md — Spec config plugins: OpenApiPlugin (OAS 3.0 + Swagger 2.0), ProtoPlugin, GraphqlPlugin, AsyncApiPlugin
- [ ] 03-03-PLAN.md — Core pipeline: merger (service dedup + spec-override), resolver (path normalization + intra-repo matching), payload assembler (ScanPayloadV1)
- [ ] 03-04-PLAN.md — Upload module: POST with retry (1s/2s/4s), response codes (202/409/429/5xx), file fallback; fault-tolerance wiring (FTOL-02/03/04)
- [ ] 03-05-PLAN.md — Scanner orchestration: default_plugins() registry, rayon parallel execution, catch_unwind per plugin, --dry-run/--output wiring, end-to-end test

### Phase 4: Language Plugins and Hardening
**Goal**: All 7 language plugins produce correct endpoints and connections from AST queries, monorepo service scoping works by nearest-ancestor, and the full scanner passes end-to-end against a polyglot fixture repo
**Depends on**: Phase 3
**Requirements**: LPLU-01, LPLU-02, LPLU-03, LPLU-04, LPLU-05, LPLU-06, LPLU-07, LPLU-08, LPLU-09, LPLU-10, LPLU-11, LPLU-12, DETQ-05, MONO-01, MONO-02, MONO-03
**Success Criteria** (what must be TRUE):
  1. A TypeScript NestJS service with @Controller("/users") and @Get("/:id") produces endpoint `GET /users/:id` in the payload (two-phase extraction works)
  2. All 7 language plugins detect their respective HTTP client calls (fetch, httpx, http.Get, RestTemplate, HttpClient, reqwest, Faraday) and produce connection findings with evidence snippets and file:line attribution
  3. A monorepo with two Dockerfiles scopes TypeScript files under service-a/ to service-a and Python files under service-b/ to service-b; shared library files under lib/ are unscoped
  4. Framework marker checks prevent full AST parsing on repos that don't contain the relevant framework (no Go plugin AST work runs on a Python-only repo)
**Plans**: 8 plans

Plans:
- [ ] 04-01-PLAN.md — AstHelper wrapper, ExtractionContext.service_roots, scope_to_service, all 7 plugin stubs
- [ ] 04-02-PLAN.md — TypeScript plugin: Express/NestJS routes (two-phase), fetch/axios clients, MQ/DB/gRPC
- [ ] 04-03-PLAN.md — Python plugin: FastAPI/Django/Flask routes, HTTP/MQ/DB/industrial/gRPC clients
- [ ] 04-04-PLAN.md — Go plugin: net/http/Gin/Echo routes, http.Get/grpc.Dial clients
- [ ] 04-05-PLAN.md — Java plugin: Spring Boot routes (two-phase), RestTemplate/WebClient/gRPC clients
- [ ] 04-06-PLAN.md — C# plugin: ASP.NET Core routes (two-phase + [controller] expansion), HttpClient/gRPC clients
- [ ] 04-07-PLAN.md — Rust plugin (Actix/Axum/reqwest/tokio-modbus) + Ruby plugin (Rails resources expansion/Faraday)
- [ ] 04-08-PLAN.md — Polyglot fixture + end-to-end integration test (MONO-01/02/03 + DETQ-05 verification)

## Progress

**Execution Order:**
Phases execute in numeric order: 1 → 2 → 3 → 4

| Phase | Plans Complete | Status | Completed |
|-------|----------------|--------|-----------|
| 1. Foundation | 0/4 | In progress | - |
| 2. Infrastructure | 0/3 | Not started | - |
| 3. Pipeline and Config Plugins | 0/5 | Not started | - |
| 4. Language Plugins and Hardening | 0/8 | Not started | - |
