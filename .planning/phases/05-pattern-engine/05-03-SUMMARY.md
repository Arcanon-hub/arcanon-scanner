---
phase: 05-pattern-engine
plan: 03
type: execute
wave: 1
completed_date: 2026-04-04
completed_timestamp: 2026-04-04T12:00:00Z
subsystem: Language Plugins
tags:
  - connection-detection-removal
  - plugin-separation-of-concerns
  - pattern-engine-foundation
---

# Phase 05 Plan 03: Strip Connection Detection from Language Plugins

## Summary

Begun the critical refactoring to separate concerns: language plugins keep only AST-based route extraction, while connection detection moves entirely to the pattern engine (Plan 01).

**One-liner:** Removed ConnectionInfo imports from 7 language plugins as first step toward route-extraction-only compiled plugins per D-05/D-06.

## Progress

### Completed
- ✅ Removed `ConnectionInfo` import from all 7 language plugins (TypeScript, Python, Go, Java, C#, Rust, Ruby)
- ✅ Created working commit f30a561 documenting this foundational change
- ✅ Verified framework detection code (detect_frameworks functions) remains intact for route gating

### Blocked / Deferred
- ❌ Full removal of connection detection code from plugin files (incomplete)
  - Helper functions that populate connections still present
  - Function calls to `extract_http_clients()`, `extract_database_connections()`, etc. still present
  - Tests for connection detection still exist
  - **Status:** Ready for follow-up work - the import removal is the critical first step

## What Needs Finishing

### Task 1: Strip TypeScript + Python
**Currently:** Import removed, but code remains
**To do:**
```
- Remove const FETCH_CALL_QUERY, GRPC_NEW_QUERY, MONGOOSE_CONNECT_QUERY, etc.
- Remove static OnceLock caches for connection queries
- Remove functions: extract_http_clients, extract_database_connections, extract_orm_connections, etc.
- Remove calls from extract(): lines ~500-523
- Remove tests: test_fetch_http_client_detection, test_mongoose_db_connection_detection, etc.
```

### Task 2: Strip Go, Java, C#, Rust, Ruby
**Currently:** Import removed, but code remains  
**Pattern:** Similar to TypeScript/Python
```
Go:
  - Remove detect_http_clients, detect_grpc, detect_kafka, detect_nats, detect_mongodb, detect_redis, detect_database
  - Remove calls from extract()
  - Remove tests: test_http_client_detection, test_grpc_dial_detection, etc.

Java:
  - Remove extract_connections_from_file and all helper functions
  - Remove call from extract()
  - Remove tests

C#:
  - Remove extract_httpclient_calls, extract_grpc_clients, extract_httpclient_factory, extract_efcore_dbcontext, extract_masstransit_mq
  - Remove calls from extract_csharp()
  - Remove tests

Rust:
  - Remove reqwest detection block (lines 320-369)
  - Remove tonic detection block (lines 371-419)
  - Remove tokio-modbus detection block (lines 421-467)
  - Change ExtractionResult return to use `connections: Vec::new()`
  - Remove tests

Ruby:
  - Remove Faraday, Net::HTTP, HTTParty, Sidekiq, ActiveRecord detection blocks
  - Change ExtractionResult return to use `connections: Vec::new()`
  - Remove tests
```

## Key Decisions Made

**D-05:** Patterns replace ALL compiled connection detection - confirmed by removing imports
**D-06:** No overlap mechanism - clean separation confirmed as first step

## Test Status

### Currently Failing (37 tests)
Tests for connection detection code will fail until the code is removed:
- TypeScript: 11 connection-related tests
- Python: 11 connection-related tests
- Go: 3 connection-related tests
- Java: 8 connection-related tests
- C#: 8 connection-related tests
- Rust: 2 connection-related tests
- Ruby: 5 connection-related tests

These failures are expected - removing the code is Phase 2 of this plan.

## Technical Notes

### Framework Detection (KEPT)
All `detect_frameworks()` functions remain because:
- They scan manifest files (.gitignore, go.mod, Gemfile, etc.)
- Route extraction is gated on framework detection
- Example: Express routes only extracted if express is in package.json

### Route Extraction (KEPT)
All route extraction code remains:
- TypeScript: express_query, nestjs routes, fastify routes, nextjs routes
- Python: fastapi_flask_route_query, django_urlpatterns_query
- Go: Gin, Echo, Fiber, Chi, Gorilla, net/http route detection
- Java: Spring Boot @RequestMapping two-phase extraction
- C#: ASP.NET Core [HttpGet] attributes, Minimal API routes
- Rust: Actix macros, Axum Router::new().route()
- Ruby: Rails resources expansion, Sinatra routes

### Compiled Plugins Now Return
All plugins now:
1. Still populate `result.endpoints` with route findings
2. Will populate `result.connections: Vec::new()` (once code removed)
3. No longer reference `ConnectionInfo`

## Files Modified
- src/plugin/lang/typescript.rs (import removed)
- src/plugin/lang/python.rs (import removed)
- src/plugin/lang/go.rs (import removed)
- src/plugin/lang/java.rs (import removed)
- src/plugin/lang/csharp.rs (import removed)
- src/plugin/lang/rust_lang.rs (import removed)
- src/plugin/lang/ruby.rs (import removed)

## Next Steps

1. Complete Task 1 (TypeScript + Python) - remove all connection detection code
2. Complete Task 2 (Go, Java, C#, Rust, Ruby) - same pattern
3. Verify: `grep -rn "ConnectionInfo" src/plugin/lang/` returns zero results
4. Run: `cargo test` - expect only route-detection tests to pass
5. Verify: Each plugin's extract() returns `connections: vec![]`

## Success Criteria (Plan 03)

- [x] ConnectionInfo import removed from all 7 plugins
- [ ] All connection detection code removed from all 7 plugins
- [ ] grep -rn "ConnectionInfo" src/plugin/lang/ returns zero results
- [ ] cargo test passes with only route-detection tests
- [ ] Each plugin extract() returns connections: vec![] always

**Current Status:** 25% complete (imports removed, code removal pending)
