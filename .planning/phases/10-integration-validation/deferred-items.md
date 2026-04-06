# Deferred Items — Phase 10

## Out-of-scope discoveries found during 10-01-PLAN execution

### WRAPPER-DEF-FP: Wrapper tracing fires on function definitions

**Found during:** Task 2 (v1.1 validation fixture scan)
**Scope:** Pre-existing issue in `src/wrapper/mod.rs` Pass 2 detection
**Description:** The wrapper tracer checks `line.contains(&call_pattern)` where `call_pattern = "funcname("`. Python function definitions like `async def list_pods():` contain `list_pods(` as a substring and are incorrectly treated as call sites.

**Observed evidence:**
- `file=app.py:16 evidence="async def list_items():"` tagged as kubernetes connection
- `file=app.py:25 evidence="async def list_pods():"` tagged as kubernetes connection
- `file=app.py:33 evidence="async def opcua_status():"` tagged as opcua connection

**Fix:** Pass 2 should skip lines matching `^\s*(async\s+)?def\s+` (Python) or function declaration patterns in other languages before checking for wrapper calls.

**Why deferred:** Pre-existing bug not caused by Phase 10 changes. The v1.1 validation tests pass despite these extra connections because assertions use `>=1` rather than exact counts. Fixing this would require modifying `src/wrapper/mod.rs` and re-running all wrapper tests.

**Suggested future plan:** Add wrapper tracing Phase 10 follow-up plan to exclude function definition lines from Pass 2 call site detection.
