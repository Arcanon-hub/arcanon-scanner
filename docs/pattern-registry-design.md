# Pattern Registry — Design Document

**Created:** 2026-04-04
**Status:** Draft — needs discussion
**Authors:** Ravi, Claude

---

## Problem

The scanner hard-codes detection patterns for every library in every language as Rust code. Adding detection for a new library (e.g., redis-py, bunny, confluent-kafka) requires a code change, PR, review, rebuild, and release. This doesn't scale — there are hundreds of client libraries across 7 languages and the long tail keeps growing.

## Proposal

Move simple detection patterns (HTTP clients, MQ, DB, gRPC, industrial protocols) out of compiled Rust code and into a **pattern registry** — a standalone database that the scanner fetches from at startup.

Compiled plugins stay for the hard stuff: AST-based route extraction, two-phase prefix joining, monorepo scoping, variable resolution, merger/resolver.

## What Moves vs What Stays

### Stays in compiled Rust (needs AST or complex logic)

| Component | Why it can't be a pattern |
|-----------|--------------------------|
| Express/NestJS/Fastify/Next.js route extraction | tree-sitter AST queries with method+path capture |
| FastAPI/Django/Flask route extraction | Decorator AST parsing |
| Gin/Echo/Fiber/Chi/mux route extraction | AST query on receiver.method(path) |
| Spring Boot @RequestMapping two-phase | Class prefix + method annotation joining |
| ASP.NET Core [Route] two-phase + [controller] expansion | Class-level attribute + token replacement |
| Actix/Axum/Rocket route macros | Rust macro AST parsing |
| Rails routes.rb + resources expansion | Keyword → 7 RESTful routes static table |
| .NET Minimal API (MapGet/MapPost) | Line scanning with method name parsing |
| Monorepo scoping (scope_to_service) | Path::ancestors() algorithm |
| Variable resolution chain | .env → compose → k8s priority layering |
| Merger / Resolver / Payload assembler | Core pipeline logic |
| Framework marker detection | package.json/go.mod/Gemfile scanning |

### Moves to pattern registry (content-gate + line-scan)

| Detection Type | Current approach | Pattern equivalent |
|---------------|-----------------|-------------------|
| HTTP clients (fetch, axios, got, requests, httpx, etc.) | content.contains + line scan | import_gate + call_pattern |
| DB clients (mongoose, pg, redis, sqlx, etc.) | content.contains + line scan | import_gate + call_pattern |
| MQ clients (kafkajs, pika, amqplib, sarama, etc.) | content.contains + line scan | import_gate + call_pattern |
| gRPC clients (grpc.Dial, ServiceStub, etc.) | content.contains + line scan | import_gate + call_pattern |
| Industrial protocols (pymodbus, opcua, etc.) | content.contains + line scan | import_gate + call_pattern |
| Cloud services (future: SQS, Pub/Sub, DynamoDB) | Not yet implemented | import_gate + call_pattern |

## Pattern Registry Architecture

### Dedicated database — NOT the hub user/scan DB

The pattern registry is a **separate, public, read-only data source**. It has no user data, no scan results, no auth tokens. Reasons:

1. **Different access pattern** — scanner reads patterns at startup, doesn't write. Hub DB handles user writes.
2. **Different security model** — patterns are public/shared. User data is private/scoped.
3. **Different scaling** — patterns are a small, cacheable dataset (~100KB). Hub DB scales with users/scans.
4. **Different update cadence** — patterns update weekly/monthly. Hub DB updates per-scan.
5. **Simpler operations** — pattern DB can be a static file, a CDN-hosted JSON, or a simple read-replica. No need for the full hub infrastructure.

### Deployment options (pick one)

| Option | Infra | Latency | Complexity |
|--------|-------|---------|-----------|
| **A. Static JSON on CDN** | S3/GCS bucket + CloudFront/CDN | ~50ms | Lowest |
| **B. Simple API service** | Single container + PostgreSQL | ~100ms | Low |
| **C. GitHub raw file** | GitHub repo, raw.githubusercontent.com | ~200ms | Zero infra |

**Recommendation: Option A (static JSON on CDN)** for v1.

- Patterns are authored in a Git repo (version controlled, PRs for changes)
- CI builds a single `patterns.json` and uploads to S3/GCS
- Scanner fetches `https://patterns.arcanon.dev/v1/patterns.json`
- CDN caches with 1-hour TTL
- No database, no API server, no auth needed (patterns are public)

Option B makes sense later if you need org-scoped private patterns.

### Pattern schema

```json
{
  "version": "1.0",
  "updated_at": "2026-04-04T00:00:00Z",
  "patterns": [
    {
      "id": "redis-py",
      "name": "redis-py",
      "description": "Python Redis client",
      "languages": ["python"],
      "file_patterns": ["**/*.py"],
      "import_gate": ["import redis", "from redis"],
      "detections": [
        {
          "match": "Redis(",
          "kind": "connection",
          "protocol": "redis",
          "confidence": "high",
          "target_extraction": "first_string_arg"
        },
        {
          "match": "StrictRedis(",
          "kind": "connection",
          "protocol": "redis",
          "confidence": "high",
          "target_extraction": "first_string_arg"
        },
        {
          "match": "from_url(",
          "kind": "connection",
          "protocol": "redis",
          "confidence": "high",
          "target_extraction": "first_string_arg"
        }
      ]
    },
    {
      "id": "boto3-sqs",
      "name": "AWS SQS (boto3)",
      "description": "AWS Simple Queue Service via boto3",
      "languages": ["python"],
      "file_patterns": ["**/*.py"],
      "import_gate": ["boto3"],
      "detections": [
        {
          "match": "client('sqs')",
          "kind": "connection",
          "protocol": "sqs",
          "confidence": "high",
          "target_extraction": "none"
        },
        {
          "match": "send_message(",
          "kind": "connection",
          "protocol": "sqs",
          "confidence": "medium",
          "target_extraction": "named_arg:QueueUrl"
        }
      ]
    }
  ]
}
```

### Schema fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Unique pattern ID (kebab-case) |
| `name` | string | Human-readable name |
| `description` | string | What this pattern detects |
| `languages` | string[] | Which language plugins apply this (`python`, `typescript`, etc.) |
| `file_patterns` | string[] | Glob patterns for file filtering |
| `import_gate` | string[] | File must contain at least one of these strings (fast skip) |
| `detections[].match` | string | Line must contain this string |
| `detections[].kind` | string | `"connection"` or `"endpoint"` (future) |
| `detections[].protocol` | string | Free string: `redis`, `kafka`, `sqs`, `mongodb`, etc. |
| `detections[].confidence` | string | `high`, `medium`, `low` |
| `detections[].target_extraction` | string | How to extract the target name from the line |

### Target extraction strategies

| Strategy | Description | Example |
|----------|-------------|---------|
| `none` | No target extraction, just emit the connection | `client('sqs')` |
| `first_string_arg` | Extract first quoted string after the match | `Redis("redis://host")` → `redis://host` |
| `named_arg:<name>` | Extract value of named argument | `send_message(QueueUrl="...")` → URL |
| `url_hostname` | Parse URL and extract hostname | `http://user-service:3000` → `user-service` |

## Scanner-side implementation

### Startup flow

```
1. Load compiled plugins (unchanged)
2. Load embedded default patterns (compiled into binary via include_str!)
3. Try fetch remote patterns:
   GET https://patterns.arcanon.dev/v1/patterns.json
   - If success → cache to ~/.arcanon/patterns.json, use these
   - If 304 Not Modified → use cached version
   - If fail → use cached version (if exists)
   - If no cache → use embedded defaults only
4. Load .arcanon.toml [[patterns]] overrides (user-defined)
5. Merge: embedded defaults ← remote patterns ← local overrides
6. Run scan with compiled plugins + pattern engine
```

### Pattern engine (new module: src/patterns/mod.rs)

```rust
pub struct PatternRegistry {
    patterns: Vec<Pattern>,
}

pub struct Pattern {
    pub id: String,
    pub languages: Vec<String>,
    pub import_gate: Vec<String>,
    pub detections: Vec<Detection>,
}

pub struct Detection {
    pub match_str: String,
    pub kind: DetectionKind,
    pub protocol: String,
    pub confidence: Confidence,
    pub target_extraction: TargetExtraction,
}

impl PatternRegistry {
    /// Load from embedded JSON + remote + local overrides
    pub fn load(hub_url: Option<&str>, api_key: Option<&str>) -> Self;

    /// Apply patterns to a file, return findings
    pub fn apply(&self, file: &FileContext, language: &str) -> ExtractionResult;
}
```

### Integration with scanner.rs

```rust
// In scanner::run():

// Existing: run compiled plugins
let plugin_results = run_plugins(&plugins, &ctx);

// New: run pattern engine
let pattern_results = pattern_registry.apply_all(&ctx.files, &detected_languages);

// Merge both
let all_results = merge_results(plugin_results, pattern_results);
```

### Payload metadata

The scanner reports which pattern set was used:

```json
{
  "metadata": {
    "tool": "cli",
    "tool_version": "0.1.0",
    "pattern_version": "1.0",
    "pattern_source": "remote",
    "patterns_applied": 47
  }
}
```

## Migration plan

### Phase 1: Pattern engine + embedded defaults (scanner-only, no infra)

- Build `src/patterns/mod.rs` with the pattern engine
- Move the 27 missing detections + existing simple detections into `patterns/*.json`
- Embed JSON files in binary via `include_str!`
- Compiled plugins shed their content-gate + line-scan code, keep AST-only code
- `.arcanon.toml` `[[patterns]]` support for user overrides
- Hub fetch is a stub (always falls back to embedded)

### Phase 2: Remote pattern distribution (CDN)

- Create `arcanon-patterns` Git repo with all pattern JSON files
- CI pipeline: validate patterns → build patterns.json → upload to CDN
- Scanner fetches from CDN at startup with caching
- Community can contribute patterns via PRs to arcanon-patterns repo

### Phase 3: Org-scoped patterns (needs API)

- Hub endpoint: `GET /api/v1/patterns?org_id=X`
- Returns global patterns + org-specific patterns
- Org admins can add private patterns for internal libraries
- Requires auth (uses existing ARCANON_API_KEY)

## Open questions

1. **Cache TTL** — How often should the scanner check for new patterns? 1 hour? 24 hours? Every scan?

2. **Pattern versioning** — If a pattern changes (e.g., false positive fixed), how do we ensure scanners pick up the fix? Cache-busting via ETag/Last-Modified headers?

3. **Pattern testing** — How do we validate patterns before publishing? Fixture files per pattern? CI that runs patterns against test repos?

4. **Compiled plugin cleanup** — When we move detections to patterns, do we remove the Rust code entirely or keep it as a fallback? If a pattern and a compiled detector both fire, which wins?

5. **Offline mode** — CI runners without internet access need embedded defaults. Should `--offline` flag skip the remote fetch entirely?

6. **Pattern metrics** — Should the scanner report which patterns matched? This helps identify unused patterns and measure false positive rates.

7. **Endpoint patterns** — Current design focuses on connection detection. Should route detection (e.g., "detect Express routes") also be pattern-driven, or is that permanently AST-only?

---

*This document needs discussion before implementation. Key decisions: CDN vs API, cache strategy, migration timeline, cleanup policy.*
