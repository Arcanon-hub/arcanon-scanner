# Arcanon Scanner

## What This Is

A Rust CLI that statically analyzes codebases to extract service boundaries, endpoints, connections, and schemas — then uploads the results to Arcanon Hub as a `ScanPayloadV1`. It runs locally on developer machines or in CI with zero cloud dependency and zero LLM requirement.

## Core Value

The scanner must accurately detect services, endpoints, and connections across 7 languages and 8 config formats using pure static analysis (AST parsing + config file reading), producing a complete `ScanPayloadV1` that the hub can use to build service dependency graphs.

## Requirements

### Validated

(None yet — ship to validate)

### Active

- [ ] CLI with clap-based argument parsing, .arcanon.toml config support, env var fallbacks
- [ ] Git context detection via gix (repo URL, branch, commit SHA with CI fallbacks)
- [ ] Variable resolution chain (.env, docker-compose, k8s ConfigMaps)
- [ ] File discovery using ignore crate with built-in excludes, .gitignore respect, size/binary guards
- [ ] Plugin architecture with LanguagePlugin trait and compiled-in registry
- [ ] 8 config plugins: OpenAPI, proto, GraphQL, AsyncAPI, docker-compose, Kubernetes, Dockerfile, .env
- [ ] 7 language plugins: TypeScript, Python, Go, Java, C#, Rust, Ruby — framework detection + AST extraction
- [ ] tree-sitter-based AST parsing with query-based extraction for all language plugins
- [ ] Service detection from build config signals (Dockerfile, compose, package manifests, etc.)
- [ ] Endpoint detection from spec files and source AST (framework-specific route patterns)
- [ ] Connection detection: HTTP clients, gRPC, message queues, databases, industrial protocols
- [ ] Schema detection from spec files, typed request/response models, and AST
- [ ] Merger: deduplicate services, merge endpoint lists, aggregate connections across plugins
- [ ] Intra-repo resolver: match outbound calls to local endpoints by (method, normalized_path)
- [ ] ScanPayloadV1 assembly matching hub's expected JSON format
- [ ] HTTP upload to hub with auth, retry (3x exponential backoff), error handling
- [ ] --output and --dry-run modes for offline/debugging use
- [ ] Monorepo support with nearest-ancestor file-to-service scoping
- [ ] Fault-tolerant scanning (file/plugin failures don't abort the scan)
- [ ] Makefile with linting (clippy), formatting (rustfmt), and unit test targets
- [ ] GitHub Actions CI: build + test + lint for Linux amd64
- [ ] Unit tests with fixture files per plugin, resolver, merger, payload assembly, variable resolution

### Out of Scope

- LLM enhancement layer (v2 — architecture doc section 16)
- External plugin protocol via stdin/stdout JSON (v2 — architecture doc section 5)
- Incremental scanning / caching (future optimization)
- Integration tests against real open-source repos (deferred)
- macOS and Windows CI builds (Linux only for v1)
- Homebrew tap and install script distribution
- Numeric confidence scores (v1.1 payload extension)

## Context

- Arcanon Hub already exists and accepts `ScanPayloadV1` uploads at `POST /api/v1/scans/upload`
- Hub handles cross-repo connection resolution — scanner only resolves intra-repo connections
- Hub's `KNOWN_TOOLS` set already includes `"cli"` for the `tool` metadata field
- Architecture document is comprehensive: `docs/architecture.md` covers all design decisions
- Companion docs exist: `scanner-no-llm-feasibility.md`, `architecture-saas-platform.md`
- tree-sitter provides multi-language AST parsing with fault tolerance and S-expression query language
- The `ignore` crate (ripgrep's engine) handles .gitignore-aware file walking

## Constraints

- **Language**: Rust — single binary, no runtime dependencies
- **Binary size**: Target < 15MB stripped (includes all tree-sitter grammars)
- **Performance**: < 2s for 100 files, < 10s for 1,000 files, < 60s for 10,000 files
- **Memory**: < 200MB peak
- **Dependencies**: Only crates listed in architecture doc section 12 (clap, gix, tree-sitter, reqwest, serde, etc.)
- **Protocol**: Free string for connection protocols — no enum, supports any protocol name
- **Payload format**: Must match existing hub `ScanPayloadV1` schema exactly — no hub changes

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| tree-sitter for all AST parsing | Multi-language with one API, fast, fault-tolerant, query language | — Pending |
| gix over git2 for git context | Pure Rust, no libgit2 dependency, lighter weight | — Pending |
| ignore crate for file walking | Same engine as ripgrep, respects nested .gitignore | — Pending |
| Plugins compiled-in for v1 | Simpler than external plugin protocol, sufficient for initial languages | — Pending |
| Hub does cross-repo matching | Scanner doesn't need cross-repo knowledge, simpler local design | — Pending |
| Linux-only CI for v1 | Minimal CI complexity, add platforms later | — Pending |
| reqwest with rustls-tls | No OpenSSL dependency, simplifies cross-compilation | — Pending |

## Evolution

This document evolves at phase transitions and milestone boundaries.

**After each phase transition** (via `/gsd:transition`):
1. Requirements invalidated? → Move to Out of Scope with reason
2. Requirements validated? → Move to Validated with phase reference
3. New requirements emerged? → Add to Active
4. Decisions to log? → Add to Key Decisions
5. "What This Is" still accurate? → Update if drifted

**After each milestone** (via `/gsd:complete-milestone`):
1. Full review of all sections
2. Core Value check — still the right priority?
3. Audit Out of Scope — reasons still valid?
4. Update Context with current state

---
*Last updated: 2026-04-04 after initialization*
