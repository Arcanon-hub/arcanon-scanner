# Milestones

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
