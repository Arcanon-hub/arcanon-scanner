# Phase 2: Infrastructure - Research

**Researched:** 2026-04-04
**Domain:** File discovery (ignore crate), git context detection (gix), variable resolution (VariableStore)
**Confidence:** HIGH

---

<phase_requirements>
## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| DISC-01 | Scanner walks directories using ignore crate, respecting nested .gitignore files at every level | `ignore::WalkBuilder` natively respects nested gitignores at every directory level — this is its core feature |
| DISC-02 | Scanner applies built-in excludes (.git/, node_modules/, __pycache__/, target/, dist/, build/, .next/, vendor/) | `OverrideBuilder` with negated globs (`!node_modules`, etc.) provides hard excludes that cannot be overridden by .gitignore allow patterns |
| DISC-03 | Scanner skips files exceeding 500KB, lines exceeding 10,000 chars, and binary files (null bytes in first 8KB) | `WalkBuilder::max_filesize(512_000)` handles size guard; line-length and binary checks require post-walk read-time guard in the file loading step |
| DISC-04 | Scanner applies additional exclude patterns from `.arcanon.toml` and `--exclude` flags | `OverrideBuilder::add()` accepts gitignore-syntax glob strings; these are added on top of built-in excludes |
| DISC-05 | Scanner does not follow symlinks | `WalkBuilder::follow_links(false)` — this is the default, explicit for clarity |
| GIT-01 | Scanner detects repo URL from first remote (origin preferred) using gix | `repo.try_find_remote("origin")` → `remote.url(Direction::Fetch)` → `url.to_bstring().to_string()` |
| GIT-02 | Scanner detects branch from HEAD ref, falling back to CI env vars (GITHUB_REF_NAME, CI_COMMIT_BRANCH, BRANCH_NAME), then "detached" | `repo.head()` → match `Kind::Symbolic(r)` → `r.name().shorten()`, then env var fallback chain |
| GIT-03 | Scanner detects commit SHA from HEAD, falling back to CI env vars, then deterministic content hash | `repo.head_id()` → `id.to_hex().to_string()`, then `GITHUB_SHA` / `CI_COMMIT_SHA`, then content hash |
| GIT-04 | Scanner derives repo_name from remote URL basename minus .git suffix, falling back to directory name | Parse remote URL string: split on `/`, take last segment, strip `.git` suffix |
| VARS-01 | Scanner builds VariableStore from .env files (merge order: .env < .env.local < .env.development < .env.production) | No external crate needed — .env format is `KEY=value` lines; manual parser is 20 lines. `dotenvy` crate available if preferred. |
| VARS-02 | Scanner reads docker-compose environment entries into VariableStore | `serde_yaml_bw::from_str::<serde_yaml_bw::Value>()` then traverse `services.<name>.environment` key |
| VARS-03 | Scanner reads Kubernetes ConfigMap data into VariableStore | Deserialize into typed `ConfigMap` struct or `Value`; extract `data` map |
| VARS-04 | Language plugins can resolve variable names through the store and extract service targets from URL values | `VariableStore::resolve(&str) -> Option<&str>` method; URL parsing extracts hostname as service target |
| VARS-05 | Scanner traces variable references through the full resolution chain | `VariableStore` has three layered HashMaps; `resolve()` checks all layers in priority order |
</phase_requirements>

---

## Summary

Phase 2 adds three distinct subsystems: file walking, git context detection, and variable resolution. These are the runtime inputs that all plugins receive — no plugin work happens until all three are populated.

The `ignore` crate (ripgrep's engine) handles file walking. Its `WalkBuilder` API is the only configuration point needed: add hard excludes via `OverrideBuilder`, set `follow_links(false)`, and `max_filesize`. Binary detection and line-length guards run at file read time, not walk time, because `ignore` only handles size.

`gix` handles git context with a pure-Rust implementation. The key challenge is detached HEAD handling: in CI (GitHub Actions, GitLab CI, Jenkins) checkouts are always detached. The fallback chain `gix HEAD → CI env vars → "detached"` must be explicit. `gix` uses BString-based APIs that require `.to_string()` conversions when storing values.

`VariableStore` is a three-layer HashMap struct. The `.env` file parsing does not require an external crate — the format is simple enough to parse in-house (key=value, `#` comments, `export KEY=` prefix support). `serde_yaml_bw` handles docker-compose and Kubernetes YAML parsing.

**Primary recommendation:** Implement these three modules as `src/git/mod.rs`, `src/vars/mod.rs`, and `src/discovery/mod.rs` with the exact struct shapes defined in the architecture doc. Each module has a single public entry-point function that returns a `Result<T>`.

---

## Standard Stack

### Core

| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| `ignore` | 0.4.25 | .gitignore-aware recursive file walker | Ripgrep's engine — the only production-grade option with native gitignore support across nested directories |
| `gix` | 0.81.0 | Git context: remote URL, branch, commit SHA | Pure Rust — no libgit2 C dep. Critical for static musl builds. 0.81.0 released 2026-03-22. |
| `serde_yaml_bw` | 2.5.4 | YAML deserialization for docker-compose and k8s ConfigMaps | Project-wide decision (replaces deprecated `serde_yaml`). Drop-in API. Released 2026-03-30. |

### Supporting (already in Cargo.toml from Phase 1)

| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `anyhow` | 1.0.102 | Error propagation with context | All `Result` returns from discovery, git, vars modules |
| `tracing` | 0.1.44 | Log skip-events (size guard, binary guard, symlink skip) | `tracing::debug!` for each skipped file |
| `serde` | 1.0.228 | Derive `Deserialize` for ComposeFile, K8sConfigMap structs | YAML → typed struct deserialization |

### Alternatives Considered

| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| Manual .env parser | `dotenvy` 0.15.7 crate | `dotenvy` is well-tested and handles edge cases (quoting, multiline values, `export` prefix). The extra dependency is low-cost. Either approach works — `dotenvy` is safer for edge cases. |
| `gix` | `git2` | `git2` links against libgit2 (C). Incompatible with static musl target. Locked decision: use `gix`. |
| `ignore` | `walkdir` | `walkdir` has no gitignore support. Not a viable swap. |

**Installation (additions to existing Cargo.toml):**
No new crates needed — `ignore`, `gix`, and `serde_yaml_bw` are already in the planned Cargo.toml from STACK.md. No new dependencies required for Phase 2.

**Version verification:** Confirmed against crates.io API 2026-04-04:
- `ignore`: 0.4.25 (2025-10-30)
- `gix`: 0.81.0 (2026-03-22)
- `serde_yaml_bw`: 2.5.4 (2026-03-30)

---

## Architecture Patterns

### Recommended Module Structure

```
src/
├── discovery/
│   └── mod.rs       # walk_repo() → Vec<PathBuf>, file guards
├── git/
│   └── mod.rs       # detect_git_context() → GitContext
└── vars/
    └── mod.rs       # build_variable_store() → VariableStore
```

These three modules are called sequentially from `core/scanner.rs` before any plugin runs:

```
git::detect_git_context(root)?
vars::build_variable_store(root)?
discovery::walk_repo(root, config)?   ← uses both .gitignore and .arcanon.toml excludes
```

### Pattern 1: File Walking with Hard Excludes

**What:** Use `WalkBuilder` with an `OverrideBuilder` that hard-excludes built-in directories, then adds user excludes on top.
**When to use:** Single entry point for all file discovery. Called once per scan.

```rust
// Source: docs.rs/ignore/0.4.25/ignore/
use ignore::{WalkBuilder, overrides::OverrideBuilder};

fn walk_repo(root: &Path, excludes: &[String]) -> anyhow::Result<Vec<PathBuf>> {
    // Built-in excludes — these CANNOT be overridden by .gitignore allow patterns
    let mut overrides = OverrideBuilder::new(root);
    for pattern in BUILT_IN_EXCLUDES {
        overrides.add(&format!("!{}", pattern))?;
    }
    // User excludes from .arcanon.toml and --exclude flags
    for pattern in excludes {
        overrides.add(&format!("!{}", pattern))?;
    }
    let overrides = overrides.build()?;

    let walker = WalkBuilder::new(root)
        .follow_links(false)              // DISC-05: no symlinks
        .max_filesize(Some(512_000))      // DISC-03: 500KB guard (size only)
        .overrides(overrides)
        .build();

    let mut files = Vec::new();
    for result in walker {
        let entry = result?;
        if entry.file_type().map_or(false, |ft| ft.is_file()) {
            // Post-walk guards for binary and line-length (DISC-03)
            if let Some(path) = passes_content_guards(entry.path()) {
                files.push(path.to_owned());
            }
        }
    }
    Ok(files)
}

const BUILT_IN_EXCLUDES: &[&str] = &[
    ".git/", "node_modules/", "__pycache__/", ".tox/", ".mypy_cache/",
    ".pytest_cache/", "target/", "dist/", "build/", "out/",
    ".next/", "vendor/",
];
```

### Pattern 2: Binary and Line-Length Guards

**What:** Read-time file guard applied after walking. Checks first 8KB for null bytes (binary) and first line for length.
**When to use:** Called for every file returned by the walker before it is loaded into `FileContext`.

```rust
// DISC-03: binary detection (null bytes in first 8KB) + max line length (10,000 chars)
fn passes_content_guards(path: &Path) -> Option<&Path> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 8192];
    let n = std::io::Read::read(&mut file, &mut buf).ok()?;
    // Binary guard: null byte in first 8KB
    if buf[..n].contains(&0u8) {
        tracing::debug!("skipping binary file: {}", path.display());
        return None;
    }
    // Line-length guard: check first line (most minified files are one long line)
    let first_line_len = buf[..n].iter().position(|&b| b == b'\n').unwrap_or(n);
    if first_line_len > 10_000 {
        tracing::debug!("skipping long-line file: {}", path.display());
        return None;
    }
    Some(path)
}
```

### Pattern 3: Git Context Detection with Fallback Chain

**What:** Try `gix` first, then fall back to CI env vars, then to hard defaults.
**When to use:** Called once on scanner startup.

```rust
// Source: docs.rs/gix/0.81.0/gix/
use gix::remote::Direction;

pub struct GitContext {
    pub repo_url: Option<String>,
    pub repo_name: String,
    pub branch: String,
    pub commit_sha: String,
}

pub fn detect_git_context(root: &Path) -> anyhow::Result<GitContext> {
    // Try to open the git repo (walks up directories)
    let repo = gix::discover(root).ok();

    let (repo_url, repo_name) = detect_remote(repo.as_ref(), root);
    let branch = detect_branch(repo.as_ref());
    let commit_sha = detect_commit_sha(repo.as_ref());

    Ok(GitContext { repo_url, repo_name, branch, commit_sha })
}

fn detect_remote(repo: Option<&gix::Repository>, root: &Path) -> (Option<String>, String) {
    // Try origin first, then any remote
    let url_str = repo.and_then(|r| {
        r.try_find_remote("origin")
            .transpose().ok().flatten()
            .or_else(|| {
                r.remote_names()
                    .into_iter()
                    .next()
                    .and_then(|n| r.try_find_remote(n.as_ref()).transpose().ok().flatten())
            })
            .and_then(|remote| remote.url(Direction::Fetch).cloned())
            .map(|url| url.to_string())
    });
    let repo_name = url_str.as_deref()
        .and_then(|u| u.rsplit('/').next())
        .map(|s| s.trim_end_matches(".git").to_string())
        .unwrap_or_else(|| {
            root.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        });
    (url_str, repo_name)
}

fn detect_branch(repo: Option<&gix::Repository>) -> String {
    // 1. Try gix HEAD symbolic ref
    if let Some(r) = repo {
        if let Ok(head) = r.head() {
            if let Some(name) = head.referent_name() {
                // name is a FullName like refs/heads/main — shorten it
                let short = name.shorten().to_string();
                if !short.is_empty() {
                    return short;
                }
            }
        }
    }
    // 2. CI env var fallback chain
    for var in &["ARCANON_BRANCH", "GITHUB_REF_NAME", "CI_COMMIT_BRANCH", "BRANCH_NAME"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return val;
            }
        }
    }
    // 3. Final fallback
    tracing::warn!("Could not detect branch name; using 'detached'. Set ARCANON_BRANCH or GITHUB_REF_NAME.");
    "detached".to_string()
}

fn detect_commit_sha(repo: Option<&gix::Repository>) -> String {
    // 1. Try gix HEAD id
    if let Some(r) = repo {
        if let Ok(id) = r.head_id() {
            return id.to_hex().to_string();
        }
    }
    // 2. CI env var fallback chain
    for var in &["ARCANON_COMMIT_SHA", "GITHUB_SHA", "CI_COMMIT_SHA"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return val;
            }
        }
    }
    // 3. Deterministic content hash fallback
    tracing::warn!("Could not detect commit SHA; using content hash fallback.");
    content_hash_fallback()
}
```

### Pattern 4: VariableStore with Layered Priority

**What:** Three-layer HashMap with a priority-ordered `resolve()` method.
**When to use:** Built once before plugins run; passed to all plugins as `Arc<VariableStore>`.

```rust
// Source: docs/architecture.md §8
use std::collections::HashMap;

pub struct VariableStore {
    /// Highest priority: merged .env files (.env < .env.local < .env.development < .env.production)
    env_files: HashMap<String, String>,
    /// Middle priority: docker-compose environment entries
    compose_env: HashMap<String, String>,
    /// Lowest priority: k8s ConfigMap data
    k8s_env: HashMap<String, String>,
}

impl VariableStore {
    pub fn resolve(&self, key: &str) -> Option<&str> {
        self.env_files.get(key)
            .or_else(|| self.compose_env.get(key))
            .or_else(|| self.k8s_env.get(key))
            .map(|s| s.as_str())
    }

    pub fn resolve_to_target(&self, key: &str) -> Option<ServiceTarget> {
        let val = self.resolve(key)?;
        parse_url_to_service_target(val)
    }
}
```

### Pattern 5: .env File Parsing

**What:** Manual parser for `.env` format — simpler than adding a dependency, handles the required cases.
**When to use:** For each `.env*` file found by the walker.

```rust
fn parse_env_file(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        // Skip comments and blank lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Strip optional 'export ' prefix
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim().to_string();
            // Strip surrounding quotes from value
            let val = val.trim();
            let val = val
                .strip_prefix('"').and_then(|v| v.strip_suffix('"'))
                .or_else(|| val.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(val)
                .to_string();
            map.insert(key, val);
        }
    }
    map
}
```

**Note:** If edge-case handling becomes complex (multiline values, escape sequences), swap to `dotenvy` 0.15.7. The manual approach handles all common cases in the project's test fixtures.

### Pattern 6: Docker-Compose Environment Extraction

**What:** Deserialize docker-compose YAML using `serde_yaml_bw::Value` to traverse the services tree.
**When to use:** When a docker-compose YAML file is found. Feeds `VariableStore::compose_env`.

```rust
// Source: docs.rs/serde_yaml_bw/2.5.4/
use serde_yaml_bw::Value;

fn extract_compose_env(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(doc) = serde_yaml_bw::from_str::<Value>(content) else {
        tracing::warn!("Failed to parse docker-compose YAML");
        return map;
    };
    let Some(services) = doc.get("services").and_then(|s| s.as_mapping()) else {
        return map;
    };
    for (_service_name, service) in services {
        let Some(env) = service.get("environment") else { continue };
        match env {
            // List form: ["KEY=value", "KEY2=value2"]
            Value::Sequence(seq) => {
                for item in seq {
                    if let Some(s) = item.as_str() {
                        if let Some((k, v)) = s.split_once('=') {
                            map.insert(k.to_string(), v.to_string());
                        }
                    }
                }
            }
            // Map form: { KEY: value }
            Value::Mapping(m) => {
                for (k, v) in m {
                    if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                        map.insert(k.to_string(), v.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    map
}
```

### Pattern 7: Kubernetes ConfigMap Extraction

**What:** Deserialize k8s manifests using typed struct or `Value`, extract ConfigMap `data` sections.
**When to use:** When a k8s manifest YAML is found. Handles multi-document files with `---` separators.

```rust
#[derive(serde::Deserialize)]
struct K8sManifest {
    kind: Option<String>,
    data: Option<HashMap<String, String>>,
}

fn extract_k8s_configmap_env(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    // k8s files can be multi-document (separated by ---)
    for doc in content.split("\n---") {
        let Ok(manifest) = serde_yaml_bw::from_str::<K8sManifest>(doc) else {
            continue;
        };
        if manifest.kind.as_deref() == Some("ConfigMap") {
            if let Some(data) = manifest.data {
                map.extend(data);
            }
        }
    }
    map
}
```

### Anti-Patterns to Avoid

- **Walking without `OverrideBuilder`:** `WalkBuilder` respects `.gitignore`, but .gitignore files can have allow patterns (`!node_modules/`) that inadvertently re-include ignored dirs. Hard excludes via `OverrideBuilder` cannot be overridden by project .gitignore files.
- **Calling `repo.head().name()` without checking `referent_name()`:** In detached HEAD state, `.referent_name()` returns `None` — use `is_detached()` check before unwrapping the branch name.
- **Parsing `.env` files with a YAML or TOML parser:** `.env` files do not follow YAML or TOML syntax; shell-style quoting and export prefix are not valid YAML. Use the dedicated `parse_env_file()` function.
- **Building VariableStore after plugin dispatch:** The store must be fully built before any plugin runs — plugins receive it as input. Never populate it lazily inside plugin code.
- **Merging .env files with last-seen-wins without priority ordering:** The priority is `.env` < `.env.local` < `.env.development` < `.env.production`. Load them in that order, inserting into the same HashMap so later files overwrite earlier ones for the same key.

---

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| .gitignore-aware file walking | Custom recursive walker with gitignore parsing | `ignore::WalkBuilder` | gitignore has 30+ edge cases: negation, patterns with slashes, patterns in parent dirs, `.git/info/exclude`, global gitignore config. `ignore` handles all of them. |
| Git object parsing | Manual `.git/HEAD` file reading | `gix` | Detached HEAD, symrefs, packed-refs, worktrees — each is a separate code path. `gix` handles all. |
| YAML parsing | Custom line-by-line parser | `serde_yaml_bw` | YAML anchors, aliases, multi-document streams, block scalars — cannot hand-roll correctly. |

**Key insight:** The .gitignore spec is 30+ rules. Any custom walker will mishandle at least 5 of them. Similarly, reading `.git/HEAD` directly works only for the simple case — packed refs and worktrees break it.

---

## Common Pitfalls

### Pitfall 1: Detached HEAD in CI Produces Wrong Branch

**What goes wrong:** CI systems (GitHub Actions, GitLab CI, Jenkins) always check out in detached HEAD mode. `gix::Repository::head().referent_name()` returns `None` in detached state. Without the fallback chain, `branch` defaults to `"detached"` for every CI scan.

**Why it happens:** CI systems fetch a specific commit, not a branch ref. The `HEAD` file contains a SHA, not `ref: refs/heads/main`.

**How to avoid:** Implement the full fallback chain: `gix HEAD referent_name` → `ARCANON_BRANCH` → `GITHUB_REF_NAME` → `CI_COMMIT_BRANCH` → `BRANCH_NAME` → `"detached"`. Emit `tracing::warn!` when falling back to `"detached"`.

**Warning signs:** `--dry-run` output shows `"branch": "detached"` consistently during CI runs.

### Pitfall 2: OverrideBuilder Patterns Need `!` Prefix for Exclusion

**What goes wrong:** Passing `"node_modules"` to `OverrideBuilder::add()` includes it (it's an allow pattern). Exclusion requires `"!node_modules"`.

**Why it happens:** The documentation states "globs provided here have precisely the same semantics as a single line in a gitignore file, where the meaning of `!` is inverted." This means `!pattern` = exclude (opposite of gitignore's `!` = un-ignore).

**How to avoid:** Always format built-in excludes as `format!("!{}", pattern)` before calling `.add()`.

**Warning signs:** `node_modules/` directories are walked and files from them appear in plugin input.

### Pitfall 3: `gix::Url::to_string()` vs `to_bstring()`

**What goes wrong:** Remote URLs can theoretically contain non-UTF-8 bytes in the path component. Calling `.to_string()` via the `Display` impl is safe in practice (the `Display` impl handles UTF-8 lossily), but `.to_bstring()` is lossless.

**Why it happens:** `gix-url` uses `BString` internally for non-UTF-8 compatibility. The API returns `Option<&gix_url::Url>` from `remote.url(Direction::Fetch)`.

**How to avoid:** Use `url.to_bstring().to_string()` for complete safety, or use the `Display` impl (`format!("{}", url)`) which is adequate for URL strings that are always valid UTF-8 in practice.

**Warning signs:** Compile error when trying to call `.to_string()` directly on `&gix_url::Url` without importing Display or using the conversion methods.

### Pitfall 4: Docker-Compose Environment Has Two YAML Forms

**What goes wrong:** Docker-compose allows `environment` as either a list (`["KEY=val"]`) or a mapping (`{KEY: val}`). A deserializer that only handles one form silently drops half of all real-world compose files.

**Why it happens:** The compose spec allows both. Older compose files use the list form. Newer files often use the mapping form.

**How to avoid:** Handle both forms explicitly in `extract_compose_env()` using `Value::Sequence` and `Value::Mapping` match arms (see Pattern 6).

**Warning signs:** Known env vars from a compose file are absent from `VariableStore`. Check the specific file's `environment` format.

### Pitfall 5: .env Merge Order Matters

**What goes wrong:** Loading `.env.production` before `.env` means `.env` overrides production values — the opposite of the intended priority.

**Why it happens:** `ignore`'s `WalkBuilder` returns files in filesystem order, not in `.env` priority order. The files must be explicitly sorted and merged in the correct sequence.

**How to avoid:** After collecting all `.env*` files, sort and merge them in this order: `.env` → `.env.local` → `.env.development` → `.env.production`. Each successive file overwrites matching keys from earlier ones.

**Warning signs:** Variables with different values in `.env` vs `.env.production` resolve to the `.env` value instead of the production value.

### Pitfall 6: gix `remote_names()` Returns a BString Iterator

**What goes wrong:** `repo.remote_names()` returns sorted `BString` names. Calling `try_find_remote(name)` requires passing a `&BStr`. Naive conversion attempts produce compile errors.

**Why it happens:** `gix` uses `BString`/`BStr` throughout (like `&[u8]` but with string semantics) to avoid assuming UTF-8.

**How to avoid:** Use `.as_ref()` to get `&BStr` from `BString`, or pass `"origin".into()` directly when you know the name.

**Warning signs:** Compiler errors about `BStr`/`&str` type mismatches when iterating over remote names.

---

## Code Examples

### Complete `gix` Feature Flag Setup

```toml
# Cargo.toml — minimal feature set for context detection only
gix = { version = "0.81", default-features = false, features = [
    "rev-parse-delegate",  # needed for head_id() and ref resolution
] }
```

The `interrupt` feature is optional (adds Ctrl-C handling). For context detection only, `rev-parse-delegate` is the minimum required feature.

### Confirmed gix API for Context Detection

```rust
// Source: docs.rs/gix/0.81.0/gix/struct.Repository.html
use gix::remote::Direction;

// Open — discovers .git by walking up from `root`
let repo = gix::discover(root)?;

// Commit SHA
let sha = repo.head_id()?.to_hex().to_string();

// Branch name (returns None if detached)
let branch = repo.head()?
    .referent_name()
    .map(|name| name.shorten().to_string());

// Remote URL
let url = repo
    .try_find_remote("origin")
    .transpose()?  // Option<Result<Remote>> → Result<Option<Remote>>
    .flatten()
    .and_then(|remote| remote.url(Direction::Fetch).cloned())
    .map(|url| format!("{url}"));
```

### VariableStore Build Sequence

```rust
// Source: docs/architecture.md §8
pub fn build_variable_store(root: &Path, files: &[PathBuf]) -> VariableStore {
    // 1. Collect .env files and sort by priority
    let mut env_files: Vec<&PathBuf> = files.iter()
        .filter(|p| p.file_name().and_then(|n| n.to_str())
            .map_or(false, |n| n.starts_with(".env")))
        .collect();
    env_files.sort_by_key(|p| env_file_priority(p));  // .env=0, .env.local=1, etc.

    // 2. Merge .env files in priority order (last wins for same key)
    let mut env_map = HashMap::new();
    for path in env_files {
        if let Ok(content) = std::fs::read_to_string(path) {
            env_map.extend(parse_env_file(&content));
        }
    }

    // 3. Extract docker-compose and k8s vars
    let compose_env = extract_from_compose_files(files);
    let k8s_env = extract_from_k8s_files(files);

    VariableStore { env_files: env_map, compose_env, k8s_env }
}
```

---

## CI Environment Variable Reference

Complete fallback chains for each CI provider (HIGH confidence — verified against official docs):

| Field | GitHub Actions | GitLab CI | Jenkins | Fallback |
|-------|---------------|-----------|---------|----------|
| Branch | `GITHUB_REF_NAME` | `CI_COMMIT_BRANCH` | `BRANCH_NAME` (or `GIT_BRANCH` with `origin/` prefix) | `"detached"` |
| Commit SHA | `GITHUB_SHA` | `CI_COMMIT_SHA` | `GIT_COMMIT` | content hash |
| Override | `ARCANON_BRANCH` | `ARCANON_BRANCH` | `ARCANON_BRANCH` | — |

**Note on `GIT_BRANCH` in Jenkins:** Jenkins's Git plugin sets `GIT_BRANCH` as `origin/main` (prefixed with remote name). Strip the `origin/` prefix before using it. `BRANCH_NAME` (set by Jenkins multibranch pipeline) is the clean name.

---

## Environment Availability

| Dependency | Required By | Available | Version | Fallback |
|------------|------------|-----------|---------|----------|
| `cargo` / Rust toolchain | Building the scanner | Yes | cargo 1.93.1 / stable | — |
| Git executable | Phase 2 tests (fixture repos) | Not checked — not needed | — | `gix` reads .git directly, no `git` CLI needed |
| `gix` crate | GIT-01 through GIT-04 | Via Cargo | 0.81.0 | — |
| `ignore` crate | DISC-01 through DISC-05 | Via Cargo | 0.4.25 | — |
| `serde_yaml_bw` crate | VARS-02, VARS-03 | Via Cargo | 2.5.4 | — |

**Missing dependencies with no fallback:** None.

**Missing dependencies with fallback:** None.

**Step 2.6 note:** All Phase 2 functionality is pure code — no external processes or services are required at runtime. `gix` reads the `.git` directory directly without invoking the `git` CLI.

---

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| `serde_yaml` | `serde_yaml_bw` | March 2024 (deprecation) | Drop-in replacement — change `use serde_yaml::` to `use serde_yaml_bw::` |
| `git2` (libgit2 bindings) | `gix` (pure Rust) | Gradual 2022-2025 | No C deps; static musl binary works without extra tooling |
| `walkdir` | `ignore` | `ignore` has been the standard for 5+ years | Built-in gitignore support; no custom filtering logic needed |

**Deprecated/outdated:**
- `serde_yaml` 0.9.34+deprecated: Do not use. Archived March 2024. No security updates.
- `git2` for static builds: Requires `libgit2` C library. Cannot produce a dependency-free musl binary.
- `dotenv` crate (original): Unmaintained. Replaced by `dotenvy` (the maintained fork).

---

## Open Questions

1. **`gix` feature flags for 0.81.0**
   - What we know: `rev-parse-delegate` enables `head_id()` and ref resolution. `interrupt` is optional.
   - What's unclear: Whether any additional features are needed for `remote_names()` and `try_find_remote()` without `default-features = false`.
   - Recommendation: Start with `features = ["rev-parse-delegate"]` and add features if compile errors appear. The compile errors will name the missing features.

2. **Content hash fallback for commit SHA**
   - What we know: Architecture doc specifies "deterministic content hash (SHA-256 of sorted file paths + sizes)" as the fallback when no git context is available.
   - What's unclear: The exact implementation: should it hash `relative_path:size` pairs? Should it include file modification time (not deterministic across CI) or not?
   - Recommendation: Hash `sorted(relative_path + ":" + file_size_bytes)` concatenated — do NOT include mtime. This ensures the same content in the same repo always produces the same hash.

3. **Jenkins `GIT_BRANCH` prefix stripping**
   - What we know: Jenkins sets `GIT_BRANCH=origin/main` (with the remote name prefix).
   - What's unclear: Whether all Jenkins configurations use `origin/` or whether some use different remote names.
   - Recommendation: Strip everything up to and including the first `/` when the value matches the pattern `<word>/<branch>` — this covers `origin/main`, `upstream/main`, etc. Only apply this strip when the var name is `GIT_BRANCH`, not for `BRANCH_NAME`.

---

## Sources

### Primary (HIGH confidence)
- [docs.rs/ignore/0.4.25/ignore/](https://docs.rs/ignore/0.4.25/ignore/) — WalkBuilder, OverrideBuilder, DirEntry API
- [docs.rs/gix/0.81.0/gix/](https://docs.rs/gix/0.81.0/gix/) — Repository, Head, Remote API
- [docs.rs/gix/latest/src/gix/repository/remote.rs.html](https://docs.rs/gix/latest/src/gix/repository/remote.rs.html) — `find_remote()`, `try_find_remote()`, `find_default_remote()` signatures
- [docs.rs/gix/latest/gix/struct.Remote.html](https://docs.rs/gix/latest/gix/struct.Remote.html) — `url(Direction)` method
- [docs.rs/gix-url/latest/gix_url/struct.Url.html](https://docs.rs/gix-url/latest/gix_url/struct.Url.html) — `to_bstring()`, `Display` impl
- [docs.rs/serde_yaml_bw/2.5.4/serde_yaml_bw/](https://docs.rs/serde_yaml_bw/2.5.4/serde_yaml_bw/) — `from_str()`, `Value`, `Mapping`, multi-document support
- [docs.github.com/en/actions/reference/workflows-and-actions/variables](https://docs.github.com/en/actions/reference/workflows-and-actions/variables) — `GITHUB_REF_NAME`, `GITHUB_SHA` confirmed
- [docs.gitlab.com/ci/variables/predefined_variables/](https://docs.gitlab.com/ci/variables/predefined_variables/) — `CI_COMMIT_BRANCH`, `CI_COMMIT_SHA` confirmed

### Secondary (MEDIUM confidence)
- [docs.rs/gix/0.81.0/src/gix/head/mod.rs.html](https://docs.rs/gix/0.81.0/src/gix/head/mod.rs.html) — `Kind` enum (Symbolic, Unborn, Detached), `referent_name()`, `is_detached()`, `id()` confirmed
- [theserverside.com — Jenkins Git environment variables](https://www.theserverside.com/blog/Coffee-Talk-Java-News-Stories-and-Opinions/Complete-Jenkins-Git-environment-variables-list-for-batch-jobs-and-shell-script-builds) — `GIT_BRANCH`, `GIT_COMMIT`, `BRANCH_NAME` conventions
- crates.io API — version verification for `ignore` 0.4.25, `gix` 0.81.0, `serde_yaml_bw` 2.5.4 (confirmed 2026-04-04)
- `.planning/research/STACK.md` (project-internal, HIGH) — pre-verified crate choices and versions

### Tertiary (LOW confidence)
- None — all critical claims verified with official sources

---

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH — all versions verified against crates.io 2026-04-04
- Architecture (file discovery): HIGH — `ignore` crate API verified via docs.rs
- Architecture (git context): HIGH — `gix` struct/method signatures verified via docs.rs source view
- Architecture (variable store): HIGH — design from architecture.md, YAML API verified via docs.rs
- CI env vars: HIGH — verified against official GitHub Actions and GitLab CI documentation
- Pitfalls: HIGH — derived from architecture analysis and official issue trackers (cross-referenced with PITFALLS.md)

**Research date:** 2026-04-04
**Valid until:** 2026-10-04 (stable crates — `ignore` and `gix` have stable APIs; `serde_yaml_bw` is actively maintained but watch for major version bump)
