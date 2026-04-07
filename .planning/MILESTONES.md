# Milestones

## v1.1 Detection Accuracy (Shipped: 2026-04-07)

**Phases completed:** 5 phases, 11 plans, 539 tests (+76 from v1.0)
**Timeline:** 2 days (2026-04-06 → 2026-04-07)
**Commits:** 38

**Key accomplishments:**

1. Pattern engine accuracy: file_patterns enforcement, Python docstring filtering, py-opcua narrowing
2. CDN patterns updated: py-opcua narrowed, py-kubernetes added (15M monthly PyPI downloads)
3. Library resolution dedup: one connection per (library, protocol) pair, not per import line
4. Wrapper tracing accuracy: depth cap 5→2, 28-name blocklist, Pass 2 docstring skip
5. Pattern+wrapper connection dedup: prefer pattern-engine version
6. Tech debt closed: NestJS two-phase extraction fixed, [services] config parsing implemented

**v1.0 tech debt resolved:**
- NestJS two-phase extraction — fixed (DEBT-01)
- [services] config parsing — implemented (DEBT-02)

**Tech debt carried forward:**
- Wrapper tracing fires on function definitions (`async def foo():`)

**Archives:**
- [Roadmap](milestones/v1.1-ROADMAP.md)
- [Requirements](milestones/v1.1-REQUIREMENTS.md)
- [Audit](milestones/v1.1-MILESTONE-AUDIT.md)

---

## v1.0 Arcanon Scanner (Shipped: 2026-04-05)

**Phases completed:** 7 phases, 31 plans, 463 tests
**Timeline:** 2 days (2026-04-04 → 2026-04-05)
**Rust LOC:** 14,750 | **Commits:** 168

**Key accomplishments:**

1. Full CLI with clap argument parsing, .arcanon.toml config, env var fallbacks, and tracing
2. File discovery (ignore crate), git context (gix), and variable resolution (.env/compose/k8s)
3. 8 config plugins: OpenAPI, proto, GraphQL, AsyncAPI, docker-compose, Kubernetes, Dockerfile, .env
4. 7 language plugins with tree-sitter AST: TypeScript, Python, Go, Java, C#, Rust, Ruby
5. CDN pattern engine with ETag caching — new detections without scanner releases
6. Library resolution — scans installed packages for custom SDK connection wrappers
7. Two-pass wrapper tracing with template literal normalization across 5 languages
8. End-to-end pipeline: discovery → plugins → patterns → library → wrapper → merger → resolver → payload → upload

**Tech debt carried forward:**

- NestJS two-phase extraction not working in polyglot fixture
- [services] config parsing TODO in main.rs

**Archives:**

- [Roadmap](milestones/v1.0-ROADMAP.md)
- [Requirements](milestones/v1.0-REQUIREMENTS.md)
- [Audit](milestones/v1.0-MILESTONE-AUDIT.md)

---
