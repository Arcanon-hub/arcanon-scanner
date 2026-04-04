# Library Resolution — Design Document

**Created:** 2026-04-05
**Status:** Draft — approved for Phase 6
**Authors:** Ravi, Claude

---

## Problem

The scanner detects connections from known libraries (redis, kafka, httpx, etc.) via the pattern engine. But internal/custom libraries that wrap these known libraries go undetected. If a team publishes `@acme/rpc` that internally uses `httpx`, the scanner sees `from acme_rpc import client` but doesn't know it's a connection library.

## Solution

When the scanner encounters an import of an unknown library, run the existing pattern engine on that library's installed source files. If the pattern engine finds known connection patterns inside the library, the library is a connection wrapper.

**Prerequisite:** The environment must be installed before scanning (same as running tests or linting). Document this, don't automate it.

## How It Works

```
1. Scanner scans user code
2. Finds: from edgeworks_sdk import create_client
3. "edgeworks_sdk" not in CDN patterns — unknown library
4. Looks for installed source:
   Python: venv/lib/python*/site-packages/edgeworks_sdk/
   Node:   node_modules/edgeworks-sdk/
   Go:     vendor/ or GOPATH (read go.sum for deps instead)
   Rust:   read Cargo.lock for transitive deps
   Ruby:   vendor/bundle/ or read Gemfile.lock
5. Runs EXISTING pattern engine on library source files
6. Pattern engine finds: httpx.Client(url=...) in transports/http.py
7. Result: edgeworks_sdk wraps HTTP → protocol "rest"
8. Emits ConnectionInfo for user code that imports this library
```

## No New Config Required

- No `[libraries]` section in `.arcanon.toml`
- No manual pattern definitions
- No hub changes
- Scanner reuses its existing pattern engine

The only requirement: install your dependencies before running the scanner.

```yaml
# CI
- run: pip install -r requirements.txt   # or npm ci, bundle install, etc.
- run: arcanon --dry-run
```

## Environment Discovery

| Language | Where to find installed packages |
|----------|--------------------------------|
| Python | `venv/lib/python*/site-packages/`, `.venv/lib/python*/site-packages/`, `env/lib/python*/site-packages/` |
| Node | `node_modules/` |
| Go | Read `go.sum` for transitive dep tree (no source scan needed) |
| Rust | Read `Cargo.lock` for transitive dep tree (no source scan needed) |
| Ruby | `vendor/bundle/ruby/*/gems/`, or read `Gemfile.lock` |
| Java | Read `pom.xml` / `build.gradle` resolved deps |
| C# | Read `packages.lock.json` or `.csproj` PackageReference |

For Go, Rust, Java, C# — source isn't locally installed in the project. Use lock files to check transitive dependencies instead. If a library depends on a known connection library (e.g., `acme-rpc` depends on `tonic` in Cargo.lock), infer the protocol from the transitive dep.

## What the Scanner Reports

```json
{
  "source": "user-service",
  "target": "",
  "protocol": "rest",
  "source_file": "src/main.py:15",
  "confidence": "medium",
  "extraction_method": "library_resolution:edgeworks_sdk→httpx",
  "evidence": "from edgeworks_sdk import create_client"
}
```

- `extraction_method` traces the chain: library name → underlying connection library
- `confidence: medium` because we inferred the connection indirectly
- `target` may be empty if the URL is in an env var (variable resolution may fill it)

## Hub Integration

The hub's `resolve_dangling_connections` already matches connections by `target_name` across repos. If the `edgeworks-sdk` repo is also scanned, the hub can link:

```
user-service → (uses edgeworks_sdk) → edgeworks-sdk → (uses httpx) → journal-service
```

The scanner reports what it finds. The hub connects the dots.

## Performance

Library resolution adds time per unknown import:
- Finding the package directory: ~1ms (glob)
- Scanning library source with pattern engine: ~10-50ms per library (most libraries are <100 files)
- Typical project: 5-20 unknown libraries = 0.5-1s additional

Well within the 30-60s acceptable range.

## Caching

Once the scanner determines that `edgeworks_sdk` wraps `httpx`, it can cache this mapping for the duration of the scan. No need to re-scan the library source for every file that imports it.

```rust
HashMap<String, Option<String>>  // library_name → protocol (None = not a connection lib)
```

## Edge Cases

### Library has multiple connection types
If `acme-platform-sdk` wraps both `httpx` AND `redis`, report both protocols. The hub handles multiple connections from one library.

### Library doesn't use any known connection library
Cache as `None` — not a connection library. Skip future imports of this library.

### Environment not installed
Scanner checks common venv paths. If nothing found, log at `-v` level:
```
INFO: Python venv not found — library resolution disabled. Install dependencies for full scan.
```
Continue scanning with CDN patterns only.

### Circular imports
Don't follow imports deeper than 1 level. We scan the library's own source files, not its dependencies' source files.

## Open Questions

1. **Scan depth** — Should we scan only the top-level library files, or recurse into subdirectories? (Recommendation: full recursive, libraries are small)

2. **Import tracking** — When user code does `from edgeworks_sdk import create_client; client.append(event)`, should we report `.append()` as the connection evidence, or just the import? (Recommendation: the import line, since we can't trace through method calls)

3. **Lock file vs installed source** — For Go/Rust where source isn't local, is the lock file dep tree sufficient, or do we need `go mod vendor` / `cargo vendor`? (Recommendation: lock file is sufficient for v1)

---

*This document describes Phase 6 of the Arcanon Scanner roadmap.*
