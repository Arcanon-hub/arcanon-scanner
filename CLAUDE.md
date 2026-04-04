<!-- GSD:project-start source:PROJECT.md -->
## Project

**Arcanon Scanner**

A Rust CLI that statically analyzes codebases to extract service boundaries, endpoints, connections, and schemas — then uploads the results to Arcanon Hub as a `ScanPayloadV1`. It runs locally on developer machines or in CI with zero cloud dependency and zero LLM requirement.

**Core Value:** The scanner must accurately detect services, endpoints, and connections across 7 languages and 8 config formats using pure static analysis (AST parsing + config file reading), producing a complete `ScanPayloadV1` that the hub can use to build service dependency graphs.

### Constraints

- **Language**: Rust — single binary, no runtime dependencies
- **Binary size**: Target < 15MB stripped (includes all tree-sitter grammars)
- **Performance**: < 2s for 100 files, < 10s for 1,000 files, < 60s for 10,000 files
- **Memory**: < 200MB peak
- **Dependencies**: Only crates listed in architecture doc section 12 (clap, gix, tree-sitter, reqwest, serde, etc.)
- **Protocol**: Free string for connection protocols — no enum, supports any protocol name
- **Payload format**: Must match existing hub `ScanPayloadV1` schema exactly — no hub changes
<!-- GSD:project-end -->

<!-- GSD:stack-start source:research/STACK.md -->
## Technology Stack

## Recommended Stack
### CLI Layer
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `clap` | 4.6.0 | CLI argument parsing, env var fallbacks, subcommand routing | De facto standard. Derive macro approach eliminates boilerplate. Version 4.x has been stable API since 2022. 4.6.0 released 2026-03-12. Requires Rust 1.85. |
### Git Integration
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `gix` | 0.81.0 | Branch name, commit SHA, remote URL detection | Pure Rust — no libgit2 C dependency. Eliminates dynamic linking headache for static musl binaries. The `gitoxide` project (gix) is actively maintained and production-ready. 0.81.0 released 2026-03-22. |
### AST Parsing Engine
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `tree-sitter` | 0.26.8 | Core parsing engine, S-expression query execution | Multi-language with a single API. GitHub-grade fault tolerance (parses broken files without panicking). S-expression query language avoids manual tree walking. Used in production by GitHub, Neovim, Zed, Helix, and Datadog. 0.26.8 released 2026-03-31. |
#### Grammar Crates
| Crate | Latest Version | Language | Notes |
|-------|---------------|----------|-------|
| `tree-sitter-typescript` | 0.23.2 | TypeScript + JavaScript | Also covers `.tsx` and `.jsx`. Single grammar serves all four extensions. |
| `tree-sitter-python` | 0.25.0 | Python | |
| `tree-sitter-go` | 0.25.0 | Go | |
| `tree-sitter-java` | 0.23.5 | Java | |
| `tree-sitter-c-sharp` | 0.23.1 | C# | |
| `tree-sitter-rust` | 0.24.2 | Rust (scanner scans Rust projects) | 0.24.2 released 2026-03-27. |
| `tree-sitter-ruby` | 0.23.1 | Ruby | |
### File Discovery
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `ignore` | 0.4.25 | .gitignore-aware recursive file walking | Exact engine used by ripgrep. Respects nested `.gitignore` at every directory level. Handles symlink loops, binary detection, and custom exclude patterns. No alternative comes close. |
| `globset` | 0.4.18 | Compiled glob pattern matching for plugin file_patterns | Same author (BurntSushi) as `ignore`. Used internally by `ignore` for pattern matching. Compiling globs once and reusing them is significantly faster than per-file string matching. |
### Serialization and Deserialization
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `serde` | 1.0.228 | Serialization framework | No alternative. Ecosystem standard. |
| `serde_json` | 1.0.149 | ScanPayloadV1 assembly and upload body | No alternative for JSON in Rust. |
| `toml` | 1.1.2 | `.arcanon.toml`, `Cargo.toml`, `pyproject.toml` parsing | Official TOML 1.1 spec support as of this version. Released 2026-04-01. |
#### YAML Parsing — Special Note
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `serde_yaml_bw` | 2.5.4 | YAML parsing for docker-compose, Kubernetes manifests, OpenAPI | Most actively maintained drop-in replacement for `serde_yaml`. Preserves the original API, so migration is mechanical (change import, nothing else). Panic-free parsing — hardened against Billion Laughs attack and malformed input. Released 2026-03-30 (very recently active). |
- `serde-saphyr` (0.0.23, 2026-03-30): Better performance, but 0.0.x version signals unstable API. Only 3,015 total downloads — too early for production adoption.
- `serde_yml` (0.0.12, August 2024): Fork with community traction but stalled at 0.0.12 since mid-2024.
- `yaml-rust2` (0.11.0): Low-level parser, no serde integration out of the box.
### HTTP Client
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `reqwest` | 0.13.2 | Upload ScanPayloadV1 to hub API | Production-grade async HTTP client. Version 0.13.x defaults to rustls (no OpenSSL dependency), which is exactly what this project needs for static binary distribution. Released 2026-02-06. |
### Async Runtime
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `tokio` | 1.51.0 | Async runtime for reqwest | Required by reqwest. Use the minimal feature set: `tokio = { version = "1", features = ["rt-multi-thread", "macros"] }`. Do not pull in `tokio::fs` — file I/O is synchronous in this design (rayon handles the parallelism). Released 2026-04-03. |
### Parallelism
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `rayon` | 1.11.0 | Parallel plugin execution across files | Work-stealing parallelism purpose-built for CPU-bound tasks. Converting sequential iterators to parallel iterators is a one-line change. Exactly right for "run all plugins in parallel across all files." Tokio is wrong here — tokio is for I/O-bound concurrency, not CPU-bound parallel computation. Released August 2025. |
### Error Handling
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `anyhow` | 1.0.102 | Application-level error handling | Standard for binary crates. Provides `context()`, `with_context()`, and `?` propagation without needing to define error types everywhere. Released 2026-02-20. |
### Logging and Tracing
| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `tracing` | 0.1.44 | Structured logging with span support | Standard for Rust async applications. Integrates with tokio. `--verbose` / `-v` flag maps to `tracing::Level` at runtime. |
| `tracing-subscriber` | 0.3.23 | Log formatting and output to stderr | Pair with `tracing`. `EnvFilter` supports `RUST_LOG` env var for filtering. Released 2026-03-13. |
## Cargo.toml Dependency Block
# CLI
# Git context
# AST parsing
# File walking
# Serialization
# HTTP upload
# Async runtime (for reqwest only)
# CPU-bound parallelism
# Error handling
# Logging
## What NOT to Use (and Why)
| Crate | Avoid Because |
|-------|--------------|
| `serde_yaml` | Deprecated March 2024. Author archived the repo. No security vulnerabilities currently, but no future updates. Will accumulate unsatisfied dependency requirements over time. |
| `git2` | Links against libgit2 (C). Breaks static musl builds without significant cross-compilation tooling. `gix` is the pure-Rust replacement. |
| `walkdir` | Does not respect `.gitignore`. Requires custom ignore-list logic. `ignore` crate is strictly superior and replaces `walkdir` entirely. |
| `native-tls` feature in reqwest | Introduces OpenSSL dependency on Linux. Breaks reproducible static binary distribution. Use `rustls-tls` exclusively. |
| `tokio::spawn_blocking` for AST parsing | Wrong tool. Spawn-blocking wraps sync work in a tokio pool meant for I/O. Use `rayon` for CPU-bound work. The cost of context-switching from tokio → rayon → tokio for each file is lower than blocking tokio's I/O executor. |
| `regex` for code pattern matching | Do not use regex for AST extraction — it cannot correctly handle nested structures, string escaping, or comments. tree-sitter queries are strictly correct; regex will produce false positives and false negatives. |
| `pest` or `nom` for language parsing | Requires writing grammars from scratch. tree-sitter provides production-quality grammars for all 7 target languages. Not a reasonable trade-off. |
## Alternatives Considered
| Category | Recommended | Alternative | Why Not |
|----------|-------------|-------------|---------|
| CLI parsing | `clap` 4.x | `argh`, `pico-args` | Both lack the env var integration, help generation quality, and derive ergonomics of clap. The binary size difference is minimal after LTO. |
| Git integration | `gix` | `git2` | `git2` requires libgit2 (C). Breaks static musl target. |
| AST parsing | `tree-sitter` | `syn` (Rust only), custom parsers | `syn` only handles Rust. No viable multi-language AST library with pre-built grammars except tree-sitter. |
| File walking | `ignore` | `walkdir`, `jwalk` | Neither handles `.gitignore` natively. `ignore` is the only production-grade option with gitignore support. |
| YAML parsing | `serde_yaml_bw` | `serde-saphyr` | `serde-saphyr` is at 0.0.23 with 3K total downloads — too immature for production. `serde_yaml_bw` is a drop-in replacement with active maintenance. |
| HTTP client | `reqwest` | `ureq`, `hyper` | `ureq` is synchronous (tokio already pulled in by reqwest; adding a sync HTTP client on top is incoherent). `hyper` is too low-level for this use case. |
| Parallelism | `rayon` | `tokio::spawn_blocking`, `crossbeam` | tokio spawn_blocking is for I/O-adjacent blocking work, not CPU-bound computation. crossbeam is lower-level and requires manual work distribution. Rayon's parallel iterator API is exactly right. |
| Error handling | `anyhow` | `thiserror`, `eyre` | `thiserror` is for library error types. `eyre` is a viable alternative but `anyhow` has broader ecosystem adoption and the APIs are near-identical. |
## Build Configuration
### Static Linux Binary (musl)
# .cargo/config.toml
# Install musl target
# Build stripped static binary
### Profile Settings for Release
### Minimum Supported Rust Version
## CI Configuration Notes
### GitHub Actions (Linux amd64)
- uses: dtolnay/rust-toolchain@stable
- name: Lint
- name: Format check
- name: Test
- name: Build release binary
## Sources
- crates.io API for all version numbers: https://crates.io/api/v1/crates/{crate-name} (verified 2026-04-04)
- tree-sitter releases and backwards-compatibility policy: https://github.com/tree-sitter/tree-sitter/releases
- serde_yaml deprecation announcement and community alternatives: https://users.rust-lang.org/t/serde-yaml-deprecation-alternatives/108868
- serde-saphyr crate: https://crates.io/crates/serde-saphyr
- serde_yaml_bw crate: https://crates.io/crates/serde_yaml_bw
- Datadog migration to Rust static analyzer using tree-sitter + rayon: https://www.datadoghq.com/blog/engineering/how-we-migrated-our-static-analyzer-from-java-to-rust/
- Rayon vs Tokio for CPU-bound tasks: https://www.shuttle.dev/blog/2024/04/11/using-rayon-rust
- tree-sitter ABI version compatibility: https://github.com/tree-sitter/tree-sitter/issues/3095
<!-- GSD:stack-end -->

<!-- GSD:conventions-start source:CONVENTIONS.md -->
## Conventions

Conventions not yet established. Will populate as patterns emerge during development.
<!-- GSD:conventions-end -->

<!-- GSD:architecture-start source:ARCHITECTURE.md -->
## Architecture

Architecture not yet mapped. Follow existing patterns found in the codebase.
<!-- GSD:architecture-end -->

<!-- GSD:workflow-start source:GSD defaults -->
## GSD Workflow Enforcement

Before using Edit, Write, or other file-changing tools, start work through a GSD command so planning artifacts and execution context stay in sync.

Use these entry points:
- `/gsd:quick` for small fixes, doc updates, and ad-hoc tasks
- `/gsd:debug` for investigation and bug fixing
- `/gsd:execute-phase` for planned phase work

Do not make direct repo edits outside a GSD workflow unless the user explicitly asks to bypass it.
<!-- GSD:workflow-end -->



<!-- GSD:profile-start -->
## Developer Profile

> Profile not yet configured. Run `/gsd:profile-user` to generate your developer profile.
> This section is managed by `generate-claude-profile` -- do not edit manually.
<!-- GSD:profile-end -->
