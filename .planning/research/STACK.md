# Technology Stack: Arcanon Scanner

**Project:** arcanon-scanner (Rust CLI, multi-language static code analysis)
**Researched:** 2026-04-04
**Overall confidence:** HIGH — all versions verified against crates.io as of research date

---

## Recommended Stack

### CLI Layer

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `clap` | 4.6.0 | CLI argument parsing, env var fallbacks, subcommand routing | De facto standard. Derive macro approach eliminates boilerplate. Version 4.x has been stable API since 2022. 4.6.0 released 2026-03-12. Requires Rust 1.85. |

**Confidence:** HIGH — verified crates.io 2026-04-04.

### Git Integration

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `gix` | 0.81.0 | Branch name, commit SHA, remote URL detection | Pure Rust — no libgit2 C dependency. Eliminates dynamic linking headache for static musl binaries. The `gitoxide` project (gix) is actively maintained and production-ready. 0.81.0 released 2026-03-22. |

**Why not `git2`:** `git2` links against libgit2 (C library). Static musl builds with C dependencies require extra cross-compilation setup. `gix` compiles to native Rust, zero external linking.

**Confidence:** HIGH — verified crates.io 2026-04-04.

### AST Parsing Engine

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `tree-sitter` | 0.26.8 | Core parsing engine, S-expression query execution | Multi-language with a single API. GitHub-grade fault tolerance (parses broken files without panicking). S-expression query language avoids manual tree walking. Used in production by GitHub, Neovim, Zed, Helix, and Datadog. 0.26.8 released 2026-03-31. |

**Confidence:** HIGH — verified crates.io 2026-04-04.

#### Grammar Crates

**Critical warning:** Grammar crates are published independently by different maintainers and are not always in sync with the core `tree-sitter` version. The core library is backwards-compatible (can load grammars built against older ABI versions), but not forwards-compatible (you cannot load a grammar built against a newer ABI version than the core).

**Do not pin grammar crates to the same version as the core.** Use the latest published version of each grammar crate individually, and let Cargo resolve. If a grammar crate's `tree-sitter` dependency conflicts with yours, the conflict will surface at compile time, not at runtime.

| Crate | Latest Version | Language | Notes |
|-------|---------------|----------|-------|
| `tree-sitter-typescript` | 0.23.2 | TypeScript + JavaScript | Also covers `.tsx` and `.jsx`. Single grammar serves all four extensions. |
| `tree-sitter-python` | 0.25.0 | Python | |
| `tree-sitter-go` | 0.25.0 | Go | |
| `tree-sitter-java` | 0.23.5 | Java | |
| `tree-sitter-c-sharp` | 0.23.1 | C# | |
| `tree-sitter-rust` | 0.24.2 | Rust (scanner scans Rust projects) | 0.24.2 released 2026-03-27. |
| `tree-sitter-ruby` | 0.23.1 | Ruby | |

**Grammar crates at 0.23.x are compatible with tree-sitter 0.26.x core.** The tree-sitter runtime is backwards-compatible with older ABI versions. Verified via tree-sitter's stated compatibility guarantee and community usage patterns.

**Confidence:** MEDIUM — version compatibility confirmed via tree-sitter's documented backwards-compatibility policy, but grammar crates at 0.23.x were not individually tested against 0.26.8 core as part of this research. Pin to specific versions and verify at compile time.

### File Discovery

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `ignore` | 0.4.25 | .gitignore-aware recursive file walking | Exact engine used by ripgrep. Respects nested `.gitignore` at every directory level. Handles symlink loops, binary detection, and custom exclude patterns. No alternative comes close. |
| `globset` | 0.4.18 | Compiled glob pattern matching for plugin file_patterns | Same author (BurntSushi) as `ignore`. Used internally by `ignore` for pattern matching. Compiling globs once and reusing them is significantly faster than per-file string matching. |

**Why not `walkdir`:** `walkdir` is lower-level and does not handle `.gitignore` natively. The architecture doc lists both, but `ignore` is strictly superior for this use case — it subsumes `walkdir`'s functionality while adding gitignore support. `walkdir` is not needed.

**Confidence:** HIGH — verified crates.io 2026-04-04.

### Serialization and Deserialization

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `serde` | 1.0.228 | Serialization framework | No alternative. Ecosystem standard. |
| `serde_json` | 1.0.149 | ScanPayloadV1 assembly and upload body | No alternative for JSON in Rust. |
| `toml` | 1.1.2 | `.arcanon.toml`, `Cargo.toml`, `pyproject.toml` parsing | Official TOML 1.1 spec support as of this version. Released 2026-04-01. |

**Confidence:** HIGH — verified crates.io 2026-04-04.

#### YAML Parsing — Special Note

`serde_yaml` is **deprecated** (version 0.9.34+deprecated, archived March 2024 by dtolnay). Do not use it. The maintainer has archived the GitHub repository and will not publish further releases.

**Recommendation: use `serde_yaml_bw`** (the "backward-compatible" fork).

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `serde_yaml_bw` | 2.5.4 | YAML parsing for docker-compose, Kubernetes manifests, OpenAPI | Most actively maintained drop-in replacement for `serde_yaml`. Preserves the original API, so migration is mechanical (change import, nothing else). Panic-free parsing — hardened against Billion Laughs attack and malformed input. Released 2026-03-30 (very recently active). |

**Alternatives considered:**

- `serde-saphyr` (0.0.23, 2026-03-30): Better performance, but 0.0.x version signals unstable API. Only 3,015 total downloads — too early for production adoption.
- `serde_yml` (0.0.12, August 2024): Fork with community traction but stalled at 0.0.12 since mid-2024.
- `yaml-rust2` (0.11.0): Low-level parser, no serde integration out of the box.

**Confidence:** MEDIUM — `serde_yaml_bw` is the pragmatic choice given active maintenance and API compatibility, but the YAML ecosystem is fragmented post-deprecation. Monitor for a clear community consensus winner over the next 6 months.

### HTTP Client

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `reqwest` | 0.13.2 | Upload ScanPayloadV1 to hub API | Production-grade async HTTP client. Version 0.13.x defaults to rustls (no OpenSSL dependency), which is exactly what this project needs for static binary distribution. Released 2026-02-06. |

**Feature flags to use:** `rustls-tls` (disable default `native-tls` if needed, though 0.13 defaults to rustls), `json` (for `.json()` body helper), `blocking` is not needed.

**Why not `ureq`:** `ureq` is synchronous and excellent for simple use cases. But `reqwest` is the standard for production async clients and retry logic is more natural in async context.

**Confidence:** HIGH — verified crates.io 2026-04-04.

### Async Runtime

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `tokio` | 1.51.0 | Async runtime for reqwest | Required by reqwest. Use the minimal feature set: `tokio = { version = "1", features = ["rt-multi-thread", "macros"] }`. Do not pull in `tokio::fs` — file I/O is synchronous in this design (rayon handles the parallelism). Released 2026-04-03. |

**Scope of tokio in this project:** Tokio is used only for the upload step. All scanning (file I/O + AST parsing) runs on rayon's thread pool — CPU-bound work should not run on tokio's I/O-optimized executor. The architecture naturally separates these: scan first (rayon), then upload (tokio).

**Confidence:** HIGH — verified crates.io 2026-04-04.

### Parallelism

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `rayon` | 1.11.0 | Parallel plugin execution across files | Work-stealing parallelism purpose-built for CPU-bound tasks. Converting sequential iterators to parallel iterators is a one-line change. Exactly right for "run all plugins in parallel across all files." Tokio is wrong here — tokio is for I/O-bound concurrency, not CPU-bound parallel computation. Released August 2025. |

**Confidence:** HIGH — verified crates.io 2026-04-04.

### Error Handling

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `anyhow` | 1.0.102 | Application-level error handling | Standard for binary crates. Provides `context()`, `with_context()`, and `?` propagation without needing to define error types everywhere. Released 2026-02-20. |

**Why not `thiserror`:** `thiserror` is for library crates that need structured error types for callers to match on. The scanner is a binary — it prints errors and exits. `anyhow` is the right choice.

**Confidence:** HIGH — verified crates.io 2026-04-04.

### Logging and Tracing

| Crate | Version | Purpose | Why |
|-------|---------|---------|-----|
| `tracing` | 0.1.44 | Structured logging with span support | Standard for Rust async applications. Integrates with tokio. `--verbose` / `-v` flag maps to `tracing::Level` at runtime. |
| `tracing-subscriber` | 0.3.23 | Log formatting and output to stderr | Pair with `tracing`. `EnvFilter` supports `RUST_LOG` env var for filtering. Released 2026-03-13. |

**Confidence:** HIGH — verified crates.io 2026-04-04.

---

## Cargo.toml Dependency Block

```toml
[dependencies]
# CLI
clap = { version = "4.6", features = ["derive", "env"] }

# Git context
gix = { version = "0.81", default-features = false, features = ["rev-parse-delegate", "interrupt"] }

# AST parsing
tree-sitter = "0.26"
tree-sitter-typescript = "0.23"
tree-sitter-python = "0.25"
tree-sitter-go = "0.25"
tree-sitter-java = "0.23"
tree-sitter-c-sharp = "0.23"
tree-sitter-rust = "0.24"
tree-sitter-ruby = "0.23"

# File walking
ignore = "0.4"
globset = "0.4"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "1"
serde_yaml_bw = "2.5"  # replaces deprecated serde_yaml

# HTTP upload
reqwest = { version = "0.13", default-features = false, features = ["rustls-tls", "json"] }

# Async runtime (for reqwest only)
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }

# CPU-bound parallelism
rayon = "1"

# Error handling
anyhow = "1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```

---

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

---

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

---

## Build Configuration

### Static Linux Binary (musl)

```toml
# .cargo/config.toml
[target.x86_64-unknown-linux-musl]
rustflags = ["-C", "target-feature=+crt-static"]
```

```bash
# Install musl target
rustup target add x86_64-unknown-linux-musl

# Build stripped static binary
cargo build --release --target x86_64-unknown-linux-musl
strip target/x86_64-unknown-linux-musl/release/arcanon-scanner
```

Expected stripped binary size: 10–15MB (includes all 7 tree-sitter grammars compiled in). tree-sitter grammars are the largest contributor to binary size — each grammar is ~300–800KB of compiled C code.

### Profile Settings for Release

```toml
[profile.release]
opt-level = 3
lto = true          # Link-time optimization — reduces binary size ~20%
codegen-units = 1   # Better LTO at cost of compile time
strip = true        # Strip symbols in release
```

### Minimum Supported Rust Version

The highest MSRV among required crates is **Rust 1.85** (imposed by `clap` 4.6.0 and `toml` 1.1.2).

```toml
[package]
rust-version = "1.85"
```

---

## CI Configuration Notes

### GitHub Actions (Linux amd64)

```yaml
- uses: dtolnay/rust-toolchain@stable
  with:
    toolchain: stable
    targets: x86_64-unknown-linux-musl
    components: clippy, rustfmt

- name: Lint
  run: cargo clippy -- -D warnings

- name: Format check
  run: cargo fmt --check

- name: Test
  run: cargo test

- name: Build release binary
  run: cargo build --release --target x86_64-unknown-linux-musl
```

---

## Sources

- crates.io API for all version numbers: https://crates.io/api/v1/crates/{crate-name} (verified 2026-04-04)
- tree-sitter releases and backwards-compatibility policy: https://github.com/tree-sitter/tree-sitter/releases
- serde_yaml deprecation announcement and community alternatives: https://users.rust-lang.org/t/serde-yaml-deprecation-alternatives/108868
- serde-saphyr crate: https://crates.io/crates/serde-saphyr
- serde_yaml_bw crate: https://crates.io/crates/serde_yaml_bw
- Datadog migration to Rust static analyzer using tree-sitter + rayon: https://www.datadoghq.com/blog/engineering/how-we-migrated-our-static-analyzer-from-java-to-rust/
- Rayon vs Tokio for CPU-bound tasks: https://www.shuttle.dev/blog/2024/04/11/using-rayon-rust
- tree-sitter ABI version compatibility: https://github.com/tree-sitter/tree-sitter/issues/3095
