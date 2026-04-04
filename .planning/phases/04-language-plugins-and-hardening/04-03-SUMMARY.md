---
phase: 04
plan: 03
subsystem: Language Plugins and Hardening
tags:
  - Python plugin
  - Framework detection
  - Route extraction
  - Client detection
  - Industrial protocols

dependency_graph:
  requires:
    - 04-01 (AstHelper, monorepo scoping infrastructure)
  provides:
    - Complete Python language plugin with FastAPI/Django/Flask route detection
    - HTTP, MQ, DB, industrial protocol (Modbus, OPC UA, BACnet, CAN, HL7) client detection
    - gRPC stub client detection with _pb2_grpc import gating

tech_stack:
  added: []
  patterns:
    - "Framework marker detection via requirements.txt/pyproject.toml scanning"
    - "Import-gated client detection (protocol library presence check before AST parsing)"
    - "tree-sitter QueryCursor with StreamingIterator pattern for memory efficiency"
    - "OnceLock pattern for compiled query caching"

key_files:
  created: []
  modified:
    - src/plugin/lang/python.rs (944 lines, complete plugin implementation)

key_decisions:
  - "Framework markers required before route detection (LPLU-08: skip routes if no FastAPI/Django/Flask found)"
  - "Client detection runs regardless of framework markers (catches standalone scripts)"
  - "Import gates used for industrial protocols to avoid false positives on shared code"
  - "gRPC detection requires _pb2_grpc import in file (High confidence gate)"
  - "MQ detection via pika.channel.basic_publish() simple text pattern (not AST, due to channel variable scope complexity)"

requirements_completed:
  - LPLU-02
  - LPLU-08
  - LPLU-09
  - LPLU-10
  - LPLU-11
  - LPLU-12

metrics:
  duration: "~45 minutes"
  completed_date: "2026-04-04T19:05:00Z"
  tasks_completed: 2
  files_modified: 1
  lines_added: 944
  tests_added: 8
  commits: 2
---

# Phase 04 Plan 03: Python Language Plugin Implementation Summary

**Complete Python plugin with FastAPI, Django, Flask route detection and HTTP/MQ/DB/industrial protocol/gRPC client extraction.**

## Performance

- **Duration:** ~45 minutes
- **Started:** 2026-04-04T16:56:12Z
- **Completed:** 2026-04-04T19:05:00Z
- **Tasks:** 2 of 2 complete
- **Files modified:** 1
- **Lines of code:** 944

## Accomplishments

- **Framework detection (LPLU-08):** FastAPI, Django, Flask markers scanned from requirements.txt/pyproject.toml; routes skipped if no framework found
- **Route extraction (LPLU-02, LPLU-08):**
  - FastAPI/Flask: @app.get/post/put/delete/patch decorators → EndpointInfo with method, path, handler
  - Django: urlpatterns = [path('route', view)] → EndpointInfo with method=GET, path
- **Client detection (LPLU-09 through LPLU-12):**
  - HTTP clients: requests, httpx, aiohttp, urllib → protocol=rest
  - Database: asyncpg, psycopg2, motor → protocol=postgresql/mongodb
  - Message Queue: pika → protocol=amqp (simple pattern)
  - Industrial protocols: pymodbus (Modbus), opcua (OPC UA), BAC0 (BACnet), python-can (CAN), hl7apy (HL7) → protocol strings match library names
  - gRPC: _pb2_grpc imported → ServiceStub() calls detected → protocol=grpc, target_name=ServiceName
- **All findings scoped via nearest-ancestor service_roots algorithm**
- **Evidence capped at 200 chars; source_file in "relative_path:line" format**

## Task Commits

1. **Task 1: Framework markers and route detection** - `4a4f855` (feat)
   - Framework detection: FastAPI, Django, Flask marker scanning
   - FastAPI/Flask route extraction via decorated_definition query
   - Django urlpatterns extraction via assignment + list query
   - Tests: 3 passing (FastAPI route, Django route, no framework marker)

2. **Task 2: HTTP, MQ, DB, industrial protocol, and gRPC client detection** - `52e513e` (feat)
   - HTTP client detection: requests, httpx, aiohttp
   - Database client detection: asyncpg, psycopg2, motor
   - MQ detection: pika.channel.basic_publish() via text pattern
   - Industrial protocols: pymodbus, opcua, BAC0, python-can, hl7apy (all import-gated)
   - gRPC detection: _pb2_grpc import gate + ServiceStub() pattern matching
   - Tests: 5 passing (HTTP client, database, Modbus, gRPC)

**Final state:** src/plugin/lang/python.rs with 944 lines, all tests passing

## Files Created/Modified

- `src/plugin/lang/python.rs` (944 lines)
  - PythonPlugin struct implementing LanguagePlugin trait
  - FrameworkSet for FastAPI/Django/Flask detection
  - Framework detection function
  - Query compilation via OnceLock (fastapi_flask_route_query, django_urlpatterns_query, http_client_query, db_client_query, grpc_stub_query)
  - extract_fastapi_flask_routes() - decorated_definition pattern matching
  - extract_django_routes() - assignment + list pattern matching
  - extract_http_clients() - library + method filtering
  - extract_db_clients() - library-specific protocol mapping
  - extract_mq_clients() - pika simple text pattern
  - extract_industrial_protocol_clients() - all 5 protocols (Modbus, OPC UA, BACnet, CAN, HL7)
  - extract_grpc_clients() - stub name pattern matching with High confidence
  - Test module: 8 tests covering all detection types

## Decisions Made

1. **Framework markers required before route detection (LPLU-08):** Routes are framework-specific in Python (decorators vs config files). Skipping route detection for plain .py without markers avoids false positives. Client detection runs regardless (catches standalone scripts).

2. **Import gates for industrial protocols:** pymodbus, opcua, BAC0, python-can, hl7apy are low-volume but high-value. Gating on import presence prevents false positives on shared code.

3. **gRPC detection requires _pb2_grpc import:** File must contain "_pb2_grpc" before running AST queries. ServiceStub() instantiation pattern is High confidence. Service name extracted by stripping "Stub" suffix (convention).

4. **MQ detection via simple text pattern:** pika.channel.basic_publish() is complex to extract via AST (channel is variable, routing_key position varies). Simple line-by-line pattern extraction sufficient for LPLU-09 requirement.

5. **QueryCursor StreamingIterator pattern:** Used streaming_iterator::StreamingIterator trait for lazy query matching (docs.rs/tree-sitter 0.26.8 API). Matches AST module pattern from Plan 01.

6. **scope_to_service on all findings:** Every EndpointInfo and ConnectionInfo has source_service from nearest-ancestor algorithm. Unscoped files emit tracing::warn! for visibility.

## Deviations from Plan

### Auto-fixed Issues

**1. [Rule 3 - Blocking issue] streaming_iterator trait not in scope for QueryMatches.next()**
- **Found during:** Task 1, testing
- **Issue:** QueryMatches (from tree-sitter) requires StreamingIterator trait for .next() method, but trait wasn't in scope
- **Fix:** Added `use streaming_iterator::StreamingIterator;` in extraction functions before using matches.next()
- **Files modified:** src/plugin/lang/python.rs (two functions: extract_fastapi_flask_routes, extract_django_routes)
- **Verification:** cargo test compiles without errors; QueryMatches iterator pattern now matches AstHelper module pattern from 04-01
- **Committed in:** 4a4f855, 52e513e

**2. [Rule 2 - Missing critical functionality] OnceLock import missing for query compilation**
- **Found during:** Task 1, code organization
- **Issue:** OnceLock pattern used for query caching but import not initially included
- **Fix:** Added `use std::sync::OnceLock;` at module level
- **Files modified:** src/plugin/lang/python.rs
- **Verification:** All queries compile and cache correctly via OnceLock::get_or_init()
- **Committed in:** 4a4f855

**3. [Rule 2 - Missing critical functionality] tree_sitter::Language conversion for python_language**
- **Found during:** Task 1, parser initialization
- **Issue:** tree_sitter_python::LANGUAGE is LanguageFn type; needs .into() conversion to Language type for parser.set_language()
- **Fix:** Added `let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();` before parser.set_language(&lang)
- **Files modified:** src/plugin/lang/python.rs (fastapi_flask_route_query, django_urlpatterns_query, main extract function)
- **Verification:** Parser initialization succeeds; tests pass
- **Committed in:** 4a4f855

All deviations resolved inline; no plan changes needed.

## Verification Checklist

- ✓ `cargo build --lib` succeeds with zero errors (Python plugin code)
- ✓ All 8 tests pass (3 from Task 1, 5 from Task 2)
- ✓ Framework detection working: FastAPI/Django/Flask markers found in manifest files
- ✓ Route detection: FastAPI @app.get() → EndpointInfo(GET, /path), Django urlpatterns → EndpointInfo(GET, path)
- ✓ HTTP client detection: requests.post() → ConnectionInfo(rest, method=POST)
- ✓ Database client detection: asyncpg.connect() → ConnectionInfo(postgresql)
- ✓ Industrial protocol detection: ModbusTcpClient() → ConnectionInfo(modbus, High confidence)
- ✓ gRPC detection: OrderServiceStub() → ConnectionInfo(grpc, target_name=OrderService)
- ✓ All source_file in "relative_path:line" format
- ✓ All evidence strings capped at 200 chars
- ✓ scope_to_service applied to every finding
- ✓ No tokio imports in src/plugin/lang/python.rs (hard boundary maintained)
- ✓ Unscoped files emit tracing::warn!

## Known Stubs

None. All client detection functions fully implemented.

## What Comes Next

- Plan 04-04 (Go plugin): Gin, Echo, Fiber routes + HTTP/gRPC/SQL clients
- Plan 04-05 (Java plugin): Spring Boot @RequestMapping, @RestController + RestTemplate/WebClient
- Plans 04-06 through 04-08: C#, Rust, Ruby plugins follow same pattern

---

**Completed:** 2026-04-04T19:05:00Z  
**Duration:** ~45 minutes  
**Status:** COMPLETE - Ready for language-specific testing and monorepo service boundary verification
