# Deferred Items — Phase 11

## Pre-existing Test Failure

**Test:** `tests/v1_1_validation.rs::fastapi_docstring_and_kubernetes`
**Failure:** "DACC-01: service-fastapi imports asyncua and calls asyncua.Client() — expected >=1 opcua connection, got 0"
**Status:** Pre-existing — was failing before plan 11-01 changes (confirmed via git stash)
**Root cause:** The py-opcua CDN pattern is not available in the offline test environment. The fixture `service-fastapi/app.py` calls `asyncua.Client()` but the pattern that matches `Client(` with `asyncua` import gate must be loaded from the CDN, which is not fetched during integration tests.
**Owner:** Phase 12 or future CDN-independent pattern injection work.
