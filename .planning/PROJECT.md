# Arcanon Scanner

## What This Is

A Rust CLI that statically analyzes codebases to extract service boundaries, endpoints, connections, and schemas — then uploads the results to Arcanon Hub as a `ScanPayloadV1`. Runs locally on developer machines or in CI with zero cloud dependency and zero LLM requirement.

Shipped v1.1 with 539 tests across 12 phases. Accurate detection across 7 languages and 8 config formats with false positive elimination in pattern matching, library resolution, and wrapper tracing.

## Core Value

Accurately detect services, endpoints, and connections across 7 languages and 8 config formats using pure static analysis (AST parsing + config file reading), producing a complete `ScanPayloadV1` that the hub can use to build service dependency graphs.

## Requirements

### Validated

- ✓ CLI with clap-based argument parsing, .arcanon.toml config, env var fallbacks — v1.0
- ✓ Git context detection via gix — v1.0
- ✓ Variable resolution chain (.env, docker-compose, k8s ConfigMaps) — v1.0
- ✓ File discovery with built-in excludes, .gitignore respect, size/binary guards — v1.0
- ✓ 15 compiled plugins (8 config + 7 language) with AST extraction — v1.0
- ✓ CDN pattern engine with ETag caching — v1.0
- ✓ Library resolution, wrapper tracing, monorepo scoping — v1.0
- ✓ Merger, resolver, ScanPayloadV1, upload with retry — v1.0
- ✓ CI: lint/test/build for Linux/macOS/Windows — v1.0
- ✓ Pattern engine accuracy: file_patterns enforcement, docstring filtering — v1.1
- ✓ Library resolution dedup (one per library, not per import line) — v1.1
- ✓ Wrapper tracing accuracy: depth cap 2, 28-name blocklist, docstring skip — v1.1
- ✓ Pattern+wrapper connection dedup — v1.1
- ✓ [services] config parsing (name override, ignore) — v1.1
- ✓ NestJS two-phase extraction fix — v1.1
- ✓ CDN: py-opcua narrowed, py-kubernetes added — v1.1

## Current Milestone: v1.2 Data Quality

**Goal:** Improve the quality of connection data sent to the hub — enabling extraction method filtering, dependency tracking, and higher target resolution across all languages and repo types.

**Target features:**
- Expose `extraction_method` on every `ConnectionPayload` (hub can filter 63 → 13 primary edges)
- Add `dependency` field to connections (library/framework name for dependency inventory)
- Final dedup pass before payload assembly (remove 10-15% duplicate rows)
- Env var target extraction via new `TargetExtraction::EnvDefault` strategy (6 languages + CDN patterns)
- Emit connections from `.env` file values (key-pattern matching in env.rs)
- Emit connections from Docker Compose `environment:` blocks (compose.rs)
- Parse OpenAPI `servers:` block and Swagger 2.0 `host+basePath` (openapi.rs)
- Parse Kubernetes container `env:` values for URL-like entries (kubernetes.rs)
- New Spring Boot properties plugin (`plugin/config/spring.rs`)

### Active

- ✓ DQ-01: `extraction_method` exposed in `ConnectionPayload` and serialized to hub — Phase 13
- ✓ DQ-02: `dependency` field added to `ConnectionInfo` and `ConnectionPayload`, populated across all sources — Phase 13
- ✓ DQ-03: Final dedup pass in assembler — `(source_file, protocol, target_name)` key, pattern > wrapper > library_resolution priority — Phase 13
- [ ] DQ-04: `TargetExtraction::EnvDefault` strategy in pattern engine + CDN patterns for py/ts/go/java/cs/rb/rs
- [ ] DQ-05: `.env` plugin emits connections for URL-like key matches (`*_URL`, `*_HOST`, `DATABASE_URL`, etc.)
- [ ] DQ-06: Compose plugin emits connections from `environment:` blocks with URL-like values
- [ ] DQ-07: OpenAPI plugin parses `servers:` block (v3.0) and `host+basePath` (Swagger 2.0)
- [ ] DQ-08: Kubernetes plugin parses `containers[].env` values for URL-like entries
- [ ] DQ-09: New `spring.rs` config plugin parses `application*.properties` and `application*.yml`

### Out of Scope

- LLM enhancement layer — adds cloud dependency and non-determinism
- Vulnerability/CVE detection — Snyk/Trivy's domain, not topology
- Variable indirection tracing — significant engine change, partial coverage
- External plugin protocol — v2.0
- Incremental scanning — v2.0
- Homebrew tap — v2.0

## Context

Shipped v1.1 with 539 tests, 206 commits.
Tech stack: clap, gix, tree-sitter (7 grammars), reqwest/rustls, rayon, serde.
CI on GitHub Actions: lint/test on Linux/macOS/Windows, release builds on 3 targets.
CDN patterns at patterns.arcanon.dev (22 Python patterns, 96+ total).
Install: `curl -fsSL https://arcanon.dev/install.sh | sh`
AI skills: `npx skills add arcanon-hub/arcanon-skills`

## Constraints

- **Language**: Rust — single binary, no runtime dependencies
- **Binary size**: < 25MB (includes all tree-sitter grammars + TLS)
- **Performance**: < 2s for 100 files, < 10s for 1,000 files, < 60s for 10,000 files
- **Memory**: < 200MB peak
- **Protocol**: Free string for connection protocols — no enum
- **Payload format**: Must match hub `ScanPayloadV1` schema exactly

## Key Decisions

| Decision | Rationale | Outcome |
|----------|-----------|---------|
| tree-sitter for all AST parsing | Multi-language with one API, fast, fault-tolerant | ✓ Good |
| gix over git2 for git context | Pure Rust, no libgit2 dependency | ✓ Good |
| ignore crate for file walking | Same engine as ripgrep, respects nested .gitignore | ✓ Good |
| Plugins compiled-in for v1 | Simpler than external protocol | ✓ Good |
| CDN pattern engine | Decouple detection from releases | ✓ Good |
| Wrapper depth cap 2 | Deeper chains produce diminishing returns, increasing FPs | ✓ Good — v1.1 |
| 28-name wrapper blocklist | Generic functions are never real wrappers | ✓ Good — v1.1 |
| Pattern+wrapper dedup | Prefer pattern-engine confidence over wrapper trace | ✓ Good — v1.1 |

## Evolution

This document evolves at phase transitions and milestone boundaries.

---
*Last updated: 2026-04-07 — Phase 13 complete (DQ-01, DQ-02, DQ-03 validated)*
