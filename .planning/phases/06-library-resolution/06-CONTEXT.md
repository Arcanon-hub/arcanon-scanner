# Phase 6: Library Resolution - Context

**Gathered:** 2026-04-05
**Status:** Ready for planning

<domain>
## Phase Boundary

Build a library resolution module that auto-discovers custom/internal connection libraries by running the existing pattern engine on installed package source files. When the scanner encounters a dependency not covered by CDN patterns, it finds the installed library source (or reads the lock file dep tree) and checks if the library wraps a known connection protocol. Zero config required — just install dependencies before scanning.

</domain>

<decisions>
## Implementation Decisions

### Import Extraction
- **D-01:** Read the project manifest (pyproject.toml, package.json, Cargo.toml, go.mod, Gemfile, pom.xml, .csproj) to get the dependency list. Don't scan import statements — the manifest is the source of truth.
- **D-02:** Production dependencies only. Skip devDependencies (package.json), dev-dependencies (Cargo.toml), test/dev groups (pyproject.toml). Avoids pytest, eslint, webpack etc.

### Environment Discovery
- **D-03:** Python: glob `venv/`, `.venv/`, `env/` relative to repo root, then check `VIRTUAL_ENV` env var. First match wins.
- **D-04:** Node: check `./node_modules/` (always at repo root).
- **D-05:** Ruby: check `vendor/bundle/ruby/*/gems/` or fall back to `Gemfile.lock` dep tree.
- **D-06:** Go, Rust, Java, C#: no local source — use lock file dep tree instead (see Lock File Strategy).
- **D-07:** If environment not found: log at `-v` level and continue with CDN patterns only. No crash, no error.

### Library Scan Scope
- **D-08:** Full recursive scan of the library directory. Most SDKs are 10-50 files. Run the existing pattern engine on them.
- **D-09:** Maintain a blocklist of known non-connection libraries to skip: numpy, pandas, scipy, matplotlib, django, flask, fastapi, react, vue, angular, express, nestjs, next, pytest, eslint, webpack, vite, jest, mocha. These are frameworks/tools, not connection wrappers.
- **D-10:** Cache results per-scan: `HashMap<String, Option<String>>` mapping library name → protocol (None = not a connection lib). Same library imported in 10 files = 1 scan.

### Lock File Strategy (Go/Rust/Ruby/Java/C#)
- **D-11:** Direct dependencies only — one level deep. If `acme-rpc` directly depends on `tonic`, it's a gRPC wrapper. Don't walk deeper.
- **D-12:** Parse these lock files: Cargo.lock (TOML), go.mod (direct deps), Gemfile.lock (SPECS section), pom.xml/build.gradle (XML/Groovy dep declarations).
- **D-13:** A library is a "connection wrapper" if any of its direct deps is a known connection library from the CDN pattern list.

### Claude's Discretion
- Module structure (new module vs extension of patterns/mod.rs)
- How to integrate with scanner.rs pipeline (before or after pattern engine runs)
- Blocklist format (compiled-in const array vs config file)
- Which pyproject.toml fields to parse ([project.dependencies] vs [tool.poetry.dependencies])

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Design Document
- `docs/library-resolution-design.md` — Full architecture, environment discovery, edge cases

### Scanner Integration Points
- `src/patterns/mod.rs` — Pattern engine (apply/apply_all) to reuse on library source
- `src/core/scanner.rs` — Scanner pipeline, language_map, where library resolution plugs in
- `src/discovery/mod.rs` — File walking (may need to walk library dirs too)
- `src/plugin/mod.rs` — FileContext type for creating contexts from library files

### Existing Environment Reference
- `/Users/ravichillerega/sources/management/edgeworks-sdk/` — Real-world internal SDK example
- `/Users/ravichillerega/sources/parameter-golf/.venv/` — Real Python venv for testing

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `PatternRegistry::apply_all(files, language, service_roots)` — runs all patterns on files, returns ExtractionResult. Can be called on library source files with no changes.
- `FileContext { path, relative_path, content }` — wrap library files the same way.
- `walk_repo(root, excludes)` — could be reused to walk library directories (with no excludes).

### Established Patterns
- Pattern engine already does content-gate + line-scan. Library resolution just feeds it different files.
- Scanner already loops over `language_map` per language. Library resolution runs in the same loop.
- Results merge through the same `merger::merge()` pipeline.

### Integration Points
- After pattern engine runs per language, run library resolution for unknown deps
- Feed resolved library connections into the same `pattern_results` vec
- `extraction_method: "library_resolution:{lib}→{underlying}"` distinguishes from CDN patterns

</code_context>

<specifics>
## Specific Ideas

- The `edgeworks-sdk` at `/Users/ravichillerega/sources/management/edgeworks-sdk/` is the primary test case
- A real venv exists at `/Users/ravichillerega/sources/parameter-golf/.venv/` for testing path discovery
- The blocklist should be a compiled-in const array — it changes rarely and doesn't need to be configurable

</specifics>

<deferred>
## Deferred Ideas

- Hub auto-generating patterns from scanned library repos (self-learning loop)
- Scanning global package caches (~/.cargo/registry, GOPATH) for source
- `--install-deps` flag to auto-install environment before scanning
- Resolving connection targets through library constructor analysis (needs LLM)

</deferred>

---

*Phase: 06-library-resolution*
*Context gathered: 2026-04-05*
