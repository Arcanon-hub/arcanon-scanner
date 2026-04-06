# Arcanon Scanner

## What This Is

A Rust CLI that statically analyzes codebases to extract service boundaries, endpoints, connections, and schemas — then uploads the results to Arcanon Hub as a `ScanPayloadV1`. Runs locally on developer machines or in CI with zero cloud dependency and zero LLM requirement.

Shipped v1.0 with 14,750 lines of Rust across 7 phases: CLI, file discovery, git context, variable resolution, 15 compiled plugins (8 config + 7 language), CDN pattern engine, library resolution, and two-pass wrapper tracing.

## Core Value

Accurately detect services, endpoints, and connections across 7 languages and 8 config formats using pure static analysis (AST parsing + config file reading), producing a complete `ScanPayloadV1` that the hub can use to build service dependency graphs.

## Requirements

### Validated

- ✓ CLI with clap-based argument parsing, .arcanon.toml config, env var fallbacks — v1.0
- ✓ Git context detection via gix (repo URL, branch, commit SHA with CI fallbacks) — v1.0
- ✓ Variable resolution chain (.env, docker-compose, k8s ConfigMaps) — v1.0
- ✓ File discovery using ignore crate with built-in excludes, .gitignore respect, size/binary guards — v1.0
- ✓ 8 config plugins: OpenAPI, proto, GraphQL, AsyncAPI, docker-compose, Kubernetes, Dockerfile, .env — v1.0
- ✓ 7 language plugins: TypeScript, Python, Go, Java, C#, Rust, Ruby with AST extraction — v1.0
- ✓ Service/endpoint/connection/schema detection from config and source AST — v1.0
- ✓ Merger, intra-repo resolver, ScanPayloadV1 assembly — v1.0
- ✓ HTTP upload with auth, retry, --output and --dry-run modes — v1.0
- ✓ Monorepo support with nearest-ancestor scoping — v1.0
- ✓ CDN pattern engine with ETag caching and .arcanon.toml overrides — v1.0
- ✓ Library resolution (scan installed packages for connection wrappers) — v1.0
- ✓ Two-pass wrapper tracing with template literal normalization — v1.0
- ✓ Fault-tolerant scanning (file/plugin failures don't abort) — v1.0
- ✓ CI: lint, format, test, build for Linux/macOS/Windows — v1.0

## Current Milestone: v1.1 Detection Accuracy

**Goal:** Eliminate false positive explosion in pattern/library detection and fix detection gaps.

**Target features:**
- Fix `py-opcua` CDN pattern false positives (~130 per asyncua project)
- Implement `file_patterns` filtering (parsed but never enforced)
- Fix library resolution amplification (emits per-import-line instead of per-library)
- Add Python docstring/comment filtering
- Add `py-kubernetes` CDN pattern
- NestJS two-phase extraction fix
- `[services]` config parsing implementation

### Active

- [ ] Fix CDN pattern false positives (py-opcua `Client(` too broad)
- [ ] Enforce file_patterns in pattern engine
- [ ] Deduplicate library resolution findings
- [ ] Filter Python docstrings and multi-line strings from pattern matching
- [ ] Add py-kubernetes CDN pattern
- [ ] Fix NestJS two-phase extraction in polyglot fixture
- [ ] Implement [services] config parsing

### Out of Scope

- LLM enhancement layer — adds cloud dependency and non-determinism
- Vulnerability/CVE detection — Snyk/Trivy's domain, not topology
- SARIF output — security finding format, not topology data
- Interactive/TUI mode — breaks CI piped workflows
- IDE plugin — separate distribution channel
- Daemon/watch mode — process lifecycle complexity
- Cross-repo resolution — hub's job, scanner resolves intra-repo only
- Numeric confidence scores — v2 payload extension
- Variable indirection tracing — significant engine change, partial coverage
- External plugin protocol — v2.0
- Incremental scanning — v2.0
- Homebrew tap — v2.0

## Context

Shipped v1.0 with 14,750 LOC Rust, 463 tests, 168 commits across 2 days.
Tech stack: clap, gix, tree-sitter (7 grammars), reqwest/rustls, rayon, serde.
CI runs on GitHub Actions: lint/test on Linux/macOS/Windows, release builds on 4 targets.
Binary size ~686KB stripped (well under 15MB target).
Arcanon Hub accepts uploads at `POST /api/v1/scans/upload`.

## Constraints

- **Language**: Rust — single binary, no runtime dependencies
- **Binary size**: Target < 15MB stripped (includes all tree-sitter grammars)
- **Performance**: < 2s for 100 files, < 10s for 1,000 files, < 60s for 10,000 files
- **Memory**: < 200MB peak
- **Protocol**: Free string for connection protocols — no enum
- **Payload format**: Must match hub `ScanPayloadV1` schema exactly

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| tree-sitter for all AST parsing | Multi-language with one API, fast, fault-tolerant | ✓ Good — powers all 7 language plugins |
| gix over git2 for git context | Pure Rust, no libgit2 dependency | ✓ Good — clean static builds |
| ignore crate for file walking | Same engine as ripgrep, respects nested .gitignore | ✓ Good — zero issues |
| Plugins compiled-in for v1 | Simpler than external protocol | ✓ Good — 15 plugins, no overhead |
| reqwest with rustls-tls | No OpenSSL dependency | ✓ Good — cross-platform builds work |
| CDN pattern engine | Decouple detection from releases | ✓ Good — 96 patterns, no scanner update needed |
| rayon for plugin parallelism | CPU-bound work, not I/O | ✓ Good — clean separation from tokio |
| Three-layer VariableStore | .env > compose > k8s priority | ✓ Good — covers all common config sources |

---
*Last updated: 2026-04-06 after v1.1 milestone start*
