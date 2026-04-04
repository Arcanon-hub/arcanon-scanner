# Phase 1: Foundation - Research

**Researched:** 2026-04-04
**Domain:** Rust CLI project skeleton — clap 4.x, tree-sitter 0.26.x, Makefile, GitHub Actions (musl CI)
**Confidence:** HIGH — all stack versions pre-verified in STACK.md; architecture types defined verbatim in architecture.md section 5

---

## Summary

Phase 1 is a pure skeleton phase. No business logic is implemented. The deliverable is a compiling Rust project with the complete type system, plugin trait, tree-sitter wrapper stub, CLI argument parsing, and a green CI pipeline. Every subsequent phase builds on top of this foundation without needing to revisit these concerns.

The stack is fully locked and pre-verified in `.planning/research/STACK.md`. All crate versions were confirmed against crates.io on 2026-04-04. The architecture types are specified verbatim in `docs/architecture.md` section 5. There are no open questions about what to build — the question is only how to structure the implementation tasks.

The highest-risk items in this phase are (1) the tree-sitter grammar/core version conflict, which must be resolved in the initial `Cargo.toml` before any code is written, and (2) the musl binary build, which must be validated early to avoid discovering size or linker issues post-implementation.

**Primary recommendation:** Write `Cargo.toml` first, run `cargo tree --duplicates | grep tree-sitter` before writing any source, validate `cargo build --release --target x86_64-unknown-linux-musl` produces a binary under 15MB before Phase 1 is declared done.

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| CLI-01 | `arcanon-scanner [PATH]` with sensible defaults, no config file required | clap 4.x derive macro; `#[arg(default_value = ".")]` on PATH arg |
| CLI-02 | `.arcanon.toml` config file with precedence: CLI > env > config > defaults | clap `#[arg(env = "...")]` + manual toml deserialization layered over clap defaults |
| CLI-03 | `--hub-url`, `--api-key`, `--project-slug` via flags or env vars | clap `#[arg(long, env = "ARCANON_HUB_URL")]` pattern |
| CLI-04 | `--output <FILE>` to write payload to file | `Option<PathBuf>` field on Cli struct |
| CLI-05 | `--dry-run` flag | `bool` field with `#[arg(long)]` |
| CLI-06 | `-v` / `-vv` / `-vvv` verbosity levels | clap `#[arg(short = 'v', long, action = clap::ArgAction::Count)]` → u8 |
| CLI-07 | `--version` prints version and exits | clap built-in: `#[command(version)]` on the struct |
| CLI-08 | `--plugins <LIST>` comma-separated filter | `Option<String>` parsed into `Vec<String>` at runtime |
| CLI-09 | `--exclude <GLOB>` repeatable | `Vec<String>` with `#[arg(long, action = clap::ArgAction::Append)]` |
| CLI-10 | `--repo-url`, `--branch`, `--commit-sha` git overrides | Three `Option<String>` fields with env var fallbacks |
| CLI-11 | Exit codes: 0 success, 1 upload failure, 2 invalid args | `std::process::exit(N)` on error paths; clap handles exit 2 on arg parse failure automatically |
| BLDG-01 | `make lint` → clippy --deny warnings | `cargo clippy -- -D warnings` |
| BLDG-02 | `make fmt` → rustfmt check | `cargo fmt --check` |
| BLDG-03 | `make test` → cargo test | `cargo test` |
| BLDG-04 | `make build` → debug and release | Two targets: `cargo build` and `cargo build --release` |
| BLDG-05 | GitHub Actions: lint, fmt, test on push/PR | `dtolnay/rust-toolchain@stable` + three job steps |
| BLDG-06 | GitHub Actions: build musl release binary | `cargo build --release --target x86_64-unknown-linux-musl` |
| BLDG-07 | Release profile: LTO, single codegen-unit, strip for < 15MB | `lto = "fat"`, `codegen-units = 1`, `strip = "symbols"`, `opt-level = "z"` |
</phase_requirements>

---

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `clap` | 4.6.0 | CLI arg parsing, env var fallbacks | De facto standard; derive macros eliminate boilerplate; MSRV 1.85 |
| `tree-sitter` | 0.26.8 | AST parsing core + S-expr query API | Multi-language, fault-tolerant, GitHub/Neovim/Zed production grade |
| `serde` | 1.0.228 | Serialization derive macros | Ecosystem requirement; no alternative |
| `serde_json` | 1.0.149 | JSON encoding for ScanPayloadV1 | No alternative in Rust ecosystem |
| `toml` | 1.1.2 | `.arcanon.toml` config parsing | TOML 1.1 spec support; released 2026-04-01 |
| `serde_yaml_bw` | 2.5.4 | YAML parsing (replaces deprecated serde_yaml) | Most actively maintained drop-in for archived serde_yaml |
| `anyhow` | 1.0.102 | Error handling for binary crate | Standard for binaries; `context()` and `?` propagation |
| `tracing` | 0.1.44 | Structured logging | Standard for Rust async; maps to -v/-vv/-vvv flags |
| `tracing-subscriber` | 0.3.23 | Log formatting + RUST_LOG env filter | Pairs with tracing; `EnvFilter` for runtime level control |
| `reqwest` | 0.13.2 | HTTP upload client (async) | Production-grade; rustls default (no OpenSSL) in 0.13.x |
| `tokio` | 1.51.0 | Async runtime for reqwest only | Required by reqwest; minimal features only |
| `rayon` | 1.11.0 | CPU-bound parallel plugin execution | Work-stealing; one-line parallel iterator conversion |
| `ignore` | 0.4.25 | .gitignore-aware file walking | ripgrep's engine; respects nested .gitignore |
| `globset` | 0.4.18 | Compiled glob matching for plugin patterns | Same author as `ignore`; used internally; faster than string matching |
| `gix` | 0.81.0 | Git branch/SHA/remote detection | Pure Rust; no libgit2 C dep; musl-safe |

### Grammar Crates (tree-sitter language parsers)

| Library | Version | Language |
|---------|---------|----------|
| `tree-sitter-typescript` | 0.23.2 | TypeScript + JavaScript (all four extensions) |
| `tree-sitter-python` | 0.25.0 | Python |
| `tree-sitter-go` | 0.25.0 | Go |
| `tree-sitter-java` | 0.23.5 | Java |
| `tree-sitter-c-sharp` | 0.23.1 | C# |
| `tree-sitter-rust` | 0.24.2 | Rust |
| `tree-sitter-ruby` | 0.23.1 | Ruby |

Grammar crates at 0.23.x–0.25.x are compatible with tree-sitter 0.26.x core. The tree-sitter runtime is backwards-compatible with older ABI versions (documented policy, corroborated by issue #3095). Run `cargo tree --duplicates | grep tree-sitter` in CI to catch any future split.

### What NOT to Use

| Crate | Reason |
|-------|--------|
| `serde_yaml` | Archived March 2024 by dtolnay; no future fixes |
| `git2` | Requires libgit2 (C); breaks musl static builds |
| `walkdir` | No .gitignore support; `ignore` is strictly superior |
| `native-tls` feature | Brings in OpenSSL on Linux; breaks static distribution |
| `regex` for code patterns | Cannot handle nesting/escaping/comments; use tree-sitter queries |

**Installation:**
```bash
# Install musl target (one-time per machine)
rustup target add x86_64-unknown-linux-musl

# macOS cross-compilation requires musl-cross toolchain
brew install filosottile/musl-cross/musl-cross
# Then configure .cargo/config.toml with the linker
```

**Version verification:** All versions confirmed against crates.io on 2026-04-04. Source: `.planning/research/STACK.md`.

---

## Architecture Patterns

### Project Structure

```
arcanon-scanner/
├── Cargo.toml                  # workspace root with [profile.release] settings
├── Makefile                    # lint / fmt / test / build targets
├── .cargo/
│   └── config.toml             # musl linker config
├── .github/
│   └── workflows/
│       └── ci.yml              # lint + fmt + test + musl build
├── src/
│   ├── main.rs                 # CLI entry point (clap derive)
│   ├── core/
│   │   ├── mod.rs
│   │   ├── scanner.rs          # orchestration pipeline
│   │   ├── resolver.rs         # intra-repo connection matching
│   │   ├── merger.rs           # dedup + merge ExtractionResults
│   │   └── payload.rs          # assemble ScanPayloadV1 JSON
│   ├── git/
│   │   └── mod.rs              # branch, commit, remote (gix)
│   ├── upload/
│   │   └── mod.rs              # HTTP POST + retry (reqwest/tokio)
│   ├── plugin/
│   │   ├── mod.rs              # LanguagePlugin trait + registry
│   │   ├── config/
│   │   │   └── mod.rs          # config plugin stubs (Phase 3 implements)
│   │   └── lang/
│   │       └── mod.rs          # language plugin stubs (Phase 4 implements)
│   ├── ast/
│   │   └── mod.rs              # tree-sitter wrapper (Phase 1 initializes)
│   ├── vars/
│   │   └── mod.rs              # VariableStore (Phase 2 implements)
│   └── types/
│       └── mod.rs              # ALL shared types defined here in Phase 1
```

### Pattern 1: clap 4.x Derive Macro for CLI

**What:** Define the entire CLI as a single annotated struct. clap auto-generates parsing, help text, and env var fallback.

**When to use:** Always — do not use the builder API. The derive approach is the community standard for clap 4.x.

```rust
// Source: docs/architecture.md section 3 + clap 4.x docs
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "arcanon-scanner", version, about = "Static service topology scanner")]
pub struct Cli {
    /// Root directory to scan
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Hub API endpoint
    #[arg(long, env = "ARCANON_HUB_URL")]
    pub hub_url: Option<String>,

    /// API key for upload
    #[arg(long, env = "ARCANON_API_KEY")]
    pub api_key: Option<String>,

    /// Project slug for grouping
    #[arg(long, env = "ARCANON_PROJECT_SLUG")]
    pub project_slug: Option<String>,

    /// Write payload to file instead of uploading
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Parse and print payload, don't upload
    #[arg(long)]
    pub dry_run: bool,

    /// Filter plugins (comma-separated)
    #[arg(long)]
    pub plugins: Option<String>,

    /// Exclude glob patterns (repeatable)
    #[arg(long, action = clap::ArgAction::Append)]
    pub exclude: Vec<String>,

    /// Override git remote URL
    #[arg(long, env = "ARCANON_REPO_URL")]
    pub repo_url: Option<String>,

    /// Override branch detection
    #[arg(long, env = "ARCANON_BRANCH")]
    pub branch: Option<String>,

    /// Override commit SHA detection
    #[arg(long, env = "ARCANON_COMMIT_SHA")]
    pub commit_sha: Option<String>,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace)
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}
```

**Critical detail for `-v`/`-vv`/`-vvv`:** Use `ArgAction::Count` (not `ArgAction::SetTrue`) to allow repeated flags. The `u8` value maps to tracing levels: 0=warn, 1=info, 2=debug, 3=trace.

### Pattern 2: tracing Verbosity Initialization

**What:** Map the `verbose: u8` count to a tracing level at startup.

```rust
// Source: tracing-subscriber docs + architecture.md section 3
fn init_tracing(verbose: u8) {
    let level = match verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .init();
}
```

Logs go to stderr. stdout is reserved for `--dry-run` payload output.

### Pattern 3: LanguagePlugin Trait (exact definition from architecture.md)

**What:** The plugin trait boundary that all 15 plugins (8 config + 7 language) implement.

```rust
// Source: docs/architecture.md section 5
use std::sync::Arc;
use std::path::PathBuf;

pub trait LanguagePlugin: Send + Sync {
    fn name(&self) -> &str;
    fn file_patterns(&self) -> &[&str];
    fn always_run(&self) -> bool { false }
    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult;
}

pub struct ExtractionContext {
    pub files: Vec<FileContext>,
    pub vars: Arc<VariableStore>,
    pub root: PathBuf,
}

pub struct FileContext {
    pub path: PathBuf,
    pub relative_path: String,
    pub content: Arc<str>,
}
```

**Hard boundary:** `extract()` is synchronous. No `async fn` on `LanguagePlugin`. No tokio imports in `src/plugin/`. This prevents the rayon/tokio deadlock (Pitfall 4 in PITFALLS.md).

### Pattern 4: Type Definitions (from architecture.md section 5)

**What:** All shared types live in `src/types/mod.rs`. These are defined exactly once in Phase 1 and imported everywhere.

```rust
// Source: docs/architecture.md section 5
pub struct ExtractionResult {
    pub services: Vec<ServiceInfo>,
    pub endpoints: Vec<EndpointInfo>,
    pub connections: Vec<ConnectionInfo>,
    pub schemas: Vec<SchemaInfo>,
    pub actors: Vec<ActorInfo>,
}

pub struct ServiceInfo {
    pub name: String,
    pub root_path: String,
    pub language: String,
    pub service_type: String,   // "service", "frontend", "database", "broker", "external"
    pub boundary_entry: Option<String>,
    pub confidence: Confidence,
    pub extraction_method: String,
}

pub struct EndpointInfo {
    pub service_name: String,
    pub method: String,
    pub path: String,
    pub handler: Option<String>,
    pub kind: String,           // "rest", "grpc", "graphql", "websocket"
    pub confidence: Confidence,
    pub extraction_method: String,
}

pub struct ConnectionInfo {
    pub source_service: String,
    pub target_name: String,
    pub protocol: String,       // free string — no enum
    pub method: Option<String>,
    pub path: Option<String>,
    pub source_file: String,    // "file:line" format
    pub confidence: Confidence,
    pub extraction_method: String,
    pub evidence: Option<String>,
}

pub struct SchemaInfo {
    pub name: String,
    pub role: String,           // "request", "response", "event"
    pub file: Option<String>,
    pub connection_ref: Option<String>,
    pub fields: Vec<FieldInfo>,
    pub confidence: Confidence,
    pub extraction_method: String,
}

pub struct FieldInfo {
    pub name: String,
    pub field_type: String,
    pub required: bool,
}

pub struct ActorInfo {
    // Defined for ExtractionResult completeness; content TBD in future phases
}

pub enum Confidence {
    High,
    Medium,
    Low,
}
```

**Note on `ActorInfo`:** The architecture.md shows `actors: Vec<ActorInfo>` in `ExtractionResult` and `"actors": []` in the ScanPayloadV1 example. The struct body is not specified in architecture.md. Define it as an empty struct in Phase 1 to satisfy the field; the full definition is deferred.

**Note on `VariableStore`:** The full `VariableStore` implementation is Phase 2 work. Phase 1 defines the struct with correct fields and a stub `resolve()` method that returns `None`. The type must exist in `src/vars/mod.rs` because `ExtractionContext` holds `Arc<VariableStore>`.

### Pattern 5: tree-sitter Wrapper Stub

**What:** `src/ast/mod.rs` wraps tree-sitter's `Parser` and `Query` types. Phase 1 creates the module with the core initialization pattern; language plugins (Phase 4) add queries.

```rust
// Source: tree-sitter 0.26.x Rust API
use tree_sitter::{Language, Node, Parser, Query, QueryCursor};

pub struct AstParser {
    parser: Parser,
}

impl AstParser {
    pub fn new(language: Language) -> anyhow::Result<Self> {
        let mut parser = Parser::new();
        parser.set_language(&language)
            .map_err(|e| anyhow::anyhow!("tree-sitter language init failed: {}", e))?;
        Ok(Self { parser })
    }

    pub fn parse(&mut self, source: &str) -> Option<tree_sitter::Tree> {
        self.parser.parse(source, None)
    }
}

pub fn run_query<'a>(
    query: &Query,
    root: Node<'a>,
    source: &'a [u8],
) -> Vec<tree_sitter::QueryMatch<'a, 'a>> {
    let mut cursor = QueryCursor::new();
    cursor.matches(query, root, source).collect()
}
```

**Grammar initialization pattern** (each language plugin):
```rust
// Source: tree-sitter-typescript crate docs
fn typescript_language() -> Language {
    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
}
```

Grammar crates expose a `LANGUAGE_*` constant of type `LanguageRef` in tree-sitter 0.26.x. Call `.into()` to convert to `Language`. This differs from older tree-sitter 0.22.x which used `language()` free functions.

### Pattern 6: Cargo.toml Release Profile

```toml
# Source: STACK.md build configuration section
[profile.release]
opt-level = "z"      # size-optimized (smaller than opt-level = 3)
lto = "fat"          # full LTO across all crates (most effective for size)
codegen-units = 1    # single unit enables maximum LTO effectiveness
strip = "symbols"    # strip debug symbols from binary
```

**Note:** The architecture doc says `lto = true` and `strip = true`, which are shorthand aliases. The PITFALLS.md (Pitfall 10) says `lto = "fat"` and `strip = "symbols"` for maximum effect. Use the explicit forms for clarity.

### Pattern 7: Makefile Structure

```makefile
# Source: BLDG-01..04 requirements
.PHONY: lint fmt test build

lint:
	cargo clippy -- -D warnings

fmt:
	cargo fmt --check

test:
	cargo test

build:
	cargo build
	cargo build --release
```

### Pattern 8: GitHub Actions CI

```yaml
# Source: STACK.md CI configuration section + BLDG-05/06 requirements
name: CI
on:
  push:
  pull_request:

jobs:
  ci:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
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

      - name: Build musl release binary
        run: cargo build --release --target x86_64-unknown-linux-musl

      - name: Check tree-sitter no duplicates
        run: cargo tree --duplicates | grep tree-sitter && exit 1 || exit 0

      - name: Assert binary under 15MB
        run: |
          SIZE=$(wc -c < target/x86_64-unknown-linux-musl/release/arcanon-scanner)
          [ "$SIZE" -lt 15728640 ] || (echo "Binary too large: $SIZE bytes" && exit 1)
```

**Note on musl linker on ubuntu-latest:** GitHub's `ubuntu-latest` runner supports musl cross-compilation via `cross` or `cargo-zigbuild`, but the simplest approach for CI is to install `musl-tools` and the linker. Verify the `dtolnay/rust-toolchain` action with the musl target installs correctly — the `targets:` parameter handles target installation. On Linux the musl linker is available via `apt-get install musl-tools`.

### Anti-Patterns to Avoid

- **Do not define types across multiple modules:** All shared types must live in `src/types/mod.rs`. Scattering type definitions causes circular dependency chains as the codebase grows.
- **Do not add `tokio` imports to `src/plugin/`:** The rayon/tokio deadlock (Pitfall 4) is prevented by the hard sync/async boundary. A `// NO TOKIO IMPORTS BELOW THIS LINE` comment in `src/plugin/mod.rs` serves as a guard.
- **Do not use `strip = true` shorthand in TOML:** Use `strip = "symbols"` explicitly. The shorthand is a boolean alias but the behavior is less predictable across toolchain versions.
- **Do not leave grammar crate versions open-ended:** Even though the tree-sitter ABI is backwards-compatible, pinned versions prevent Cargo from silently upgrading a grammar crate to one that pulls in a newer core ABI version (Pitfall 1 from PITFALLS.md).
- **Do not implement plugin logic in Phase 1:** All plugin `extract()` bodies return `ExtractionResult::default()` in this phase. Phase 1 is about structure, not behavior.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| .gitignore-aware file walking | Custom recursive walker | `ignore` crate | Handles nested gitignore, binary detection, symlink loops |
| CLI parsing + env var fallback | `std::env::args()` + custom parsing | `clap` derive | Generates help text, version flag, error messages, `--` separator |
| AST parsing for 7 languages | Custom parsers or regex | `tree-sitter` grammar crates | Production grammars; handles partial/broken files |
| Structured logging with levels | `eprintln!` guarded by verbosity | `tracing` + `tracing-subscriber` | Runtime level filtering, span context, async-safe |
| Error context propagation | Custom error types | `anyhow` | `context()` + `?` without boilerplate; right for binary crates |
| HTTP client with TLS | Raw `std::net::TcpStream` | `reqwest` with `rustls-tls` | Handles redirect, timeout, retry-friendly, musl-safe |
| Git metadata reading | Shell-out to `git` subprocess | `gix` | Pure Rust; no subprocess; works in environments without git in PATH |

**Key insight:** The musl target eliminates all C FFI options. Every dependency that links a C library (libgit2, OpenSSL, libssl) becomes a build problem. The stack was chosen specifically to be all-Rust or to use C code that compiles cleanly into the static binary (tree-sitter grammars are C, but they build via `build.rs` and link statically).

---

## Common Pitfalls

### Pitfall 1: tree-sitter Grammar/Core Version Split (CRITICAL)
**What goes wrong:** Two grammar crates pin different tree-sitter core versions. Rust loads both; `Language` types from different versions are incompatible. Compile error: "expected tree_sitter::Language, found a different tree_sitter::Language."
**Why it happens:** Grammar crates use `~0.24` style version pins; when one upgrades and another hasn't, Cargo resolves two core versions simultaneously.
**How to avoid:** Pin all grammar crates in `Cargo.toml` to the specific versions in STACK.md (no `^` wildcards for grammars). Add `cargo tree --duplicates | grep tree-sitter` as a CI assertion that fails the build if duplicates appear.
**Warning signs:** Compile error mentioning "different tree_sitter::Language". Or `cargo tree --duplicates` showing two `tree-sitter` entries.

### Pitfall 2: musl Build Linker Failure on macOS
**What goes wrong:** `cargo build --target x86_64-unknown-linux-musl` fails on macOS with "linker not found" because the musl cross-linker is not on PATH.
**Why it happens:** macOS does not ship a Linux musl linker. A cross-toolchain must be installed separately.
**How to avoid:** On macOS, install `filosottile/musl-cross/musl-cross` via Homebrew. Configure `.cargo/config.toml`:
```toml
[target.x86_64-unknown-linux-musl]
linker = "x86_64-linux-musl-gcc"
```
Alternatively use `cargo-zigbuild` which bundles the musl toolchain: `cargo install cargo-zigbuild && cargo zigbuild --release --target x86_64-unknown-linux-musl`.
**Warning signs:** "error: linker `cc` not found" or "linker `x86_64-linux-musl-gcc` not found" during cross-compilation.

### Pitfall 3: Binary Size Exceeds 15MB
**What goes wrong:** Default `cargo build --release` without LTO produces 25–40MB for a project with 7 compiled-C tree-sitter grammars.
**Why it happens:** Grammar C code is not subject to dead code elimination without LTO. Each grammar is 300–800KB of compiled C.
**How to avoid:** Set `lto = "fat"`, `codegen-units = 1`, `strip = "symbols"`, `opt-level = "z"` in `[profile.release]` before adding grammar crates. Verify size early — add the CI size assertion from Pattern 8.
**Warning signs:** `ls -lh target/x86_64-unknown-linux-musl/release/arcanon-scanner` shows > 15MB.

### Pitfall 4: `serde_yaml` Accidentally Used
**What goes wrong:** A developer imports `serde_yaml` instead of `serde_yaml_bw`, getting the archived/unmaintained version.
**Why it happens:** Muscle memory from existing Rust experience; `serde_yaml` still appears first in search results.
**How to avoid:** `serde_yaml_bw` is a drop-in replacement with the same API — just a different crate name. The STATE.md decision log explicitly records this choice. Add a `cargo audit` step to CI to surface any deprecated crate flags.
**Warning signs:** `Cargo.lock` contains `serde_yaml 0.9.x`.

### Pitfall 5: tokio Import in Plugin Module
**What goes wrong:** `use tokio::...` inside `src/plugin/` (or any module called from `extract()`) causes a silent deadlock when rayon worker threads block on tokio futures.
**Why it happens:** It feels natural to add async operations (tracing spans, etc.) to plugin code. But `extract()` runs on rayon's thread pool, not tokio's executor.
**How to avoid:** Add a comment gate at the top of `src/plugin/mod.rs`: `// HARD BOUNDARY: No tokio imports permitted in this module or any submodule.` The architecture enforces this at code review time.
**Warning signs:** Scanner hangs after "Scanning complete" log line but before upload. CPU at 0%.

### Pitfall 6: tree-sitter 0.26.x Grammar Init API Changed from 0.22.x
**What goes wrong:** Older examples use `tree_sitter_typescript::language()` free function. In 0.26.x this is `tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()`.
**Why it happens:** Many blog posts and Stack Overflow answers target tree-sitter 0.20–0.22.
**How to avoid:** The `LANGUAGE_*` constant (type `LanguageRef`) was introduced in tree-sitter 0.24 with `.into()` conversion to `Language`. Check current grammar crate docs on docs.rs, not tutorials.
**Warning signs:** Compile error "no function named `language` in `tree_sitter_typescript`".

---

## Code Examples

Verified patterns from architecture.md and STACK.md:

### Cargo.toml — Full Dependency Block

```toml
# Source: .planning/research/STACK.md (versions verified 2026-04-04)
[dependencies]
# CLI
clap = { version = "4.6", features = ["derive", "env"] }

# Git context (pure Rust, musl-safe)
gix = { version = "0.81", default-features = false, features = ["rev-parse-delegate", "interrupt"] }

# AST parsing core
tree-sitter = "0.26"

# Grammar crates (pinned — do NOT use ^ or * wildcards)
tree-sitter-typescript = "0.23.2"
tree-sitter-python = "0.25.0"
tree-sitter-go = "0.25.0"
tree-sitter-java = "0.23.5"
tree-sitter-c-sharp = "0.23.1"
tree-sitter-rust = "0.24.2"
tree-sitter-ruby = "0.23.1"

# File walking
ignore = "0.4"
globset = "0.4"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "1"
serde_yaml_bw = "2.5"   # DO NOT use serde_yaml — archived March 2024

# HTTP upload (rustls only — no OpenSSL)
reqwest = { version = "0.13", default-features = false, features = ["rustls-tls", "json"] }

# Async runtime (upload only — NOT for plugin code)
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }

# CPU-bound parallelism (plugin execution)
rayon = "1"

# Error handling
anyhow = "1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[profile.release]
opt-level = "z"
lto = "fat"
codegen-units = 1
strip = "symbols"
```

### .cargo/config.toml — musl Linker

```toml
# Source: STACK.md "Static Linux Binary" section
[target.x86_64-unknown-linux-musl]
rustflags = ["-C", "target-feature=+crt-static"]
# On macOS only — comment out or make conditional:
# linker = "x86_64-linux-musl-gcc"
```

### main.rs Skeleton

```rust
// Source: architecture.md section 3 + clap 4.x pattern
use clap::Parser;
use anyhow::Result;

mod core;
mod git;
mod upload;
mod plugin;
mod ast;
mod vars;
mod types;

#[derive(Parser, Debug)]
#[command(name = "arcanon-scanner", version, about = "Static service topology scanner")]
struct Cli {
    #[arg(default_value = ".")]
    path: std::path::PathBuf,

    #[arg(long, env = "ARCANON_HUB_URL")]
    hub_url: Option<String>,

    #[arg(long, env = "ARCANON_API_KEY")]
    api_key: Option<String>,

    #[arg(long, env = "ARCANON_PROJECT_SLUG")]
    project_slug: Option<String>,

    #[arg(long)]
    output: Option<std::path::PathBuf>,

    #[arg(long)]
    dry_run: bool,

    #[arg(long)]
    plugins: Option<String>,

    #[arg(long, action = clap::ArgAction::Append)]
    exclude: Vec<String>,

    #[arg(long, env = "ARCANON_REPO_URL")]
    repo_url: Option<String>,

    #[arg(long, env = "ARCANON_BRANCH")]
    branch: Option<String>,

    #[arg(long, env = "ARCANON_COMMIT_SHA")]
    commit_sha: Option<String>,

    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);
    // Phase 2+ implements the full pipeline
    Ok(())
}

fn init_tracing(verbose: u8) {
    let level = match verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .init();
}
```

### Plugin Registry Stub

```rust
// Source: docs/architecture.md section 5
use crate::types::{ExtractionContext, ExtractionResult};

pub trait LanguagePlugin: Send + Sync {
    fn name(&self) -> &str;
    fn file_patterns(&self) -> &[&str];
    fn always_run(&self) -> bool { false }
    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult;
}

pub mod config;
pub mod lang;

pub fn default_plugins() -> Vec<Box<dyn LanguagePlugin>> {
    vec![
        // Config plugins — stubs in Phase 1; implemented in Phase 3
        Box::new(config::OpenApiPlugin),
        Box::new(config::ProtoPlugin),
        Box::new(config::GraphqlPlugin),
        Box::new(config::AsyncApiPlugin),
        Box::new(config::ComposePlugin),
        Box::new(config::KubernetesPlugin),
        Box::new(config::DockerfilePlugin),
        Box::new(config::EnvPlugin),
        // Language plugins — stubs in Phase 1; implemented in Phase 4
        Box::new(lang::TypeScriptPlugin),
        Box::new(lang::PythonPlugin),
        Box::new(lang::GoPlugin),
        Box::new(lang::JavaPlugin),
        Box::new(lang::CSharpPlugin),
        Box::new(lang::RustLangPlugin),
        Box::new(lang::RubyPlugin),
    ]
}
```

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `serde_yaml` for YAML parsing | `serde_yaml_bw` (drop-in replacement) | March 2024 | Direct import swap; same API |
| `git2` for git operations | `gix` 0.81.x | Last 2 years (gix matured) | Eliminates libgit2 C dep; musl-safe |
| `tree-sitter::language()` free function | `tree-sitter::LANGUAGE_*.into()` | tree-sitter 0.24 | Grammar init pattern changed |
| `reqwest` default `native-tls` | `reqwest` 0.13 defaults to `rustls` | reqwest 0.13 (2026-02-06) | No OpenSSL needed in 0.13.x |
| `clap` builder API | `clap` derive macro | clap 3.x+ (stabilized in 4.x) | Less boilerplate; same functionality |

**Deprecated/outdated:**
- `serde_yaml`: Archived. Use `serde_yaml_bw`.
- `tree-sitter` < 0.24: `language()` free function removed/changed; grammar crate API changed.

---

## Open Questions

1. **`ActorInfo` struct fields**
   - What we know: `ExtractionResult` has `actors: Vec<ActorInfo>`. The ScanPayloadV1 example shows `"actors": []`. Architecture.md does not define the struct body.
   - What's unclear: Whether `ActorInfo` has any fields in v1, or is intentionally empty as a future extension point.
   - Recommendation: Define as `pub struct ActorInfo {}` in Phase 1 with a `// TODO: define fields when actor detection is scoped` comment. This satisfies the type requirement without guessing fields.

2. **macOS musl cross-compilation in local dev**
   - What we know: GitHub Actions on `ubuntu-latest` supports musl natively via `musl-tools`. macOS requires `filosottile/musl-cross/musl-cross` or `cargo-zigbuild`.
   - What's unclear: Whether developers work primarily on macOS and need local musl builds, or whether musl is CI-only.
   - Recommendation: Document both paths in the Makefile. Make the `build` target use the host target for local dev; add a `build-musl` target for explicit musl builds. CI always uses musl.

3. **`gix` minimal feature set**
   - What we know: STACK.md uses `default-features = false, features = ["rev-parse-delegate", "interrupt"]`. This is the minimal set for branch/SHA/remote detection.
   - What's unclear: Whether `"interrupt"` is actually required for Phase 1 (it enables Ctrl+C handling). It may be needed in Phase 2 when gix actually runs git operations.
   - Recommendation: Use the STACK.md feature set as-is; do not reduce it further. Adding features later is safe; removing them after code depends on them is a refactor.

---

## Environment Availability

| Dependency | Required By | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| Rust stable toolchain | All compilation | Check with `rustup show` | Must be >= 1.85 | Install via rustup |
| x86_64-unknown-linux-musl target | BLDG-06, CI | Install via `rustup target add` | N/A | CI handles automatically |
| musl-cross linker (macOS only) | Local musl builds | Homebrew or cargo-zigbuild | N/A | Skip local musl; rely on CI |
| GitHub Actions | BLDG-05, BLDG-06 | Standard | N/A | N/A (required for CI) |

**Note:** This is a greenfield project with no source files yet. The environment dependency is the Rust toolchain itself. No external services or databases are required for Phase 1.

---

## Validation Architecture

> `workflow.nyquist_validation` is set to `false` in `.planning/config.json`. This section is skipped per configuration.

---

## Sources

### Primary (HIGH confidence)
- `.planning/research/STACK.md` — all crate versions verified against crates.io 2026-04-04
- `.planning/research/PITFALLS.md` — pitfall analysis with GitHub issue citations
- `docs/architecture.md` — authoritative type definitions (section 5), CLI interface (section 3), project structure (section 4)
- `.planning/STATE.md` — locked decisions (serde_yaml_bw, grammar version pinning, rayon/tokio boundary, musl target settings)
- `.planning/REQUIREMENTS.md` — requirement descriptions for CLI-01..CLI-11, BLDG-01..BLDG-07

### Secondary (MEDIUM confidence)
- clap 4.x derive macro patterns: https://docs.rs/clap/4.6.0/clap/derive/index.html
- tree-sitter 0.26.x Rust API: https://docs.rs/tree-sitter/0.26.8/tree_sitter/
- tracing-subscriber EnvFilter: https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/filter/struct.EnvFilter.html

### Tertiary (LOW confidence)
- musl cross-compilation on macOS: https://github.com/FiloSottile/homebrew-musl-cross (community-maintained toolchain; working but not official Rust docs)

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — all versions pre-verified in STACK.md against crates.io 2026-04-04
- Architecture: HIGH — type definitions copied verbatim from architecture.md section 5; no inference needed
- CLI patterns: HIGH — clap 4.x derive patterns are stable and well-documented
- Pitfalls: HIGH — backed by PITFALLS.md which cites official GitHub issues
- Build/CI: HIGH for Linux CI; MEDIUM for macOS musl cross-compilation (community toolchain)

**Research date:** 2026-04-04
**Valid until:** 2026-07-04 (90 days — stable ecosystem; grammar crate versions most likely to drift)
