# Phase 5: Pattern Engine - Context

**Gathered:** 2026-04-04
**Status:** Ready for planning

<domain>
## Phase Boundary

Build a pattern engine module (`src/patterns/mod.rs`) that fetches detection patterns from `https://patterns.arcanon.dev/v1/patterns.json` at startup, caches locally, merges with `.arcanon.toml` user overrides, and applies them to produce `ConnectionInfo` findings alongside compiled plugins. Then strip all existing content-gate + line-scan connection detection code from compiled language plugins — patterns own ALL connection detection, plugins keep only AST-based route extraction.

</domain>

<decisions>
## Implementation Decisions

### Fetch & Cache
- **D-01:** Fetch patterns from CDN on every scan using ETag/If-Modified-Since conditional requests. ~50ms overhead, always up-to-date.
- **D-02:** Cache at `~/.arcanon/patterns.json`. User-level, shared across repos.
- **D-03:** On network failure, warn to stderr and fall back to cached patterns. If no cache exists, warn and continue with zero dynamic patterns (compiled plugins still run).
- **D-04:** No embedded default patterns in binary. Scanner requires network on first run or a pre-populated cache. This keeps the binary lean and ensures patterns always come from the canonical source.

### Pattern-Plugin Overlap
- **D-05:** Patterns replace ALL compiled connection detection. Strip content-gate + line-scan code from all 7 language plugins. Compiled plugins become route-extraction-only (AST queries for Express, NestJS, FastAPI, Spring Boot, etc.).
- **D-06:** No overlap mechanism needed — clean separation. Patterns own connections, plugins own routes.

### Target Extraction
- **D-07:** Simple string operations for target extraction (str::find + slicing). No regex crate dependency.
- **D-08:** Extraction strategies: `none` (no extraction), `first_string_arg` (first quoted string after match), `named_arg:key` (find key= then extract value), `url_hostname` (parse URL, extract host).
- **D-09:** When extraction fails (variable argument, no string literal), emit connection with `target_name: ""` and `confidence: Medium`. Hub can still use protocol and source_file.

### .arcanon.toml Override Format
- **D-10:** User patterns defined inline as `[[patterns]]` array in `.arcanon.toml`.
- **D-11:** Override by ID — user pattern with same ID as remote pattern replaces it entirely. New IDs add to the set.
- **D-12:** Disable list at `[scanner.patterns] disabled = ["ts-axios", "py-boto3-sqs"]`. Blocklist for unwanted remote patterns.

### Claude's Discretion
- Pattern engine module structure and internal APIs
- How to wire pattern results into the existing merger pipeline
- Order of operations in scanner.rs (fetch patterns before or after file discovery)
- Error handling for malformed pattern JSON

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Pattern Registry Design
- `docs/pattern-registry-design.md` — Full architecture, schema definition, migration plan, open questions

### Remote Pattern Source
- `https://patterns.arcanon.dev/v1/patterns.json` — Live endpoint (96 patterns, 209 detections, 7 languages)

### Scanner Core (integration points)
- `src/core/scanner.rs` — Scanner orchestration; pattern engine integrates here
- `src/core/merger.rs` — Merger that combines plugin + pattern results
- `src/core/payload.rs` — Payload assembler; needs pattern_version/pattern_source metadata fields
- `src/main.rs` — CLI entry point; passes hub_url for pattern fetch

### Existing Plugin Code (to be stripped)
- `src/plugin/lang/typescript.rs` — HTTP/MQ/DB/gRPC detection code to remove
- `src/plugin/lang/python.rs` — HTTP/MQ/DB/industrial/gRPC detection code to remove
- `src/plugin/lang/go.rs` — HTTP/MQ/DB/gRPC/NATS detection code to remove
- `src/plugin/lang/java.rs` — RestTemplate/WebClient/FeignClient/Kafka/RabbitMQ/gRPC detection code to remove
- `src/plugin/lang/csharp.rs` — HttpClient/EF Core/MassTransit/gRPC detection code to remove
- `src/plugin/lang/rust_lang.rs` — reqwest/tonic/tokio-modbus detection code to remove
- `src/plugin/lang/ruby.rs` — Faraday/HTTParty/Sidekiq/ActiveRecord/gRPC detection code to remove

### Types
- `src/types/mod.rs` — ConnectionInfo, ExtractionResult types that pattern engine must produce

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `ExtractionResult` and `ConnectionInfo` types already defined — pattern engine produces these
- `scope_to_service()` in `src/plugin/mod.rs` — pattern engine needs this for monorepo attribution
- `VariableStore` in `src/vars/mod.rs` — could be used to resolve variable-based targets (future)

### Established Patterns
- Content gate (check file contains X) + line scan (find match on line) is already the pattern used by ~60% of compiled detection code — the pattern engine just externalizes this
- `reqwest` crate already in Cargo.toml — can be reused for the CDN fetch (already used by upload module)
- `serde_json` already in Cargo.toml — used for parsing pattern JSON

### Integration Points
- `scanner.rs::run()` — pattern engine initializes here, before plugin execution
- `merger::merge()` — pattern results feed into the same merge pipeline as plugin results
- `payload.rs::assemble()` — needs new metadata fields (pattern_version, pattern_source)
- `main.rs` — hub_url/api_key passed to pattern engine for fetch URL construction

</code_context>

<specifics>
## Specific Ideas

- Pattern fetch URL: `https://patterns.arcanon.dev/v1/patterns.json` (already deployed on Vercel)
- Cache location: `~/.arcanon/patterns.json`
- The `arcanon-patterns` repo at `/Users/ravichillerega/sources/arcanon-patterns` is the source of truth for patterns
- Pattern schema matches what's already deployed (see the live endpoint)

</specifics>

<deferred>
## Deferred Ideas

- Hub-served org-scoped patterns (v3 — needs auth + org context)
- Pattern metrics/analytics (which patterns fire most, false positive rates)
- Pattern A/B testing
- Auto-updating embedded defaults from CI

</deferred>

---

*Phase: 05-pattern-engine*
*Context gathered: 2026-04-04*
