---
phase: 15
plan: "15-02"
title: "OpenAPI Servers Extraction + Kubernetes Container Env Extraction"
subsystem: config-plugins
tags: [openapi, kubernetes, connections, DQ-07, DQ-08]
requirements: [DQ-07, DQ-08]

dependency_graph:
  requires: [url_util]
  provides: [openapi-server-connections, k8s-env-connections]
  affects: [ExtractionResult.connections]

tech_stack:
  added: []
  patterns:
    - "ParseResult widened to 4-tuple adding Vec<ConnectionInfo>"
    - "openapiv3::Server.url: String accessed directly (field verified in cargo registry)"
    - "Swagger 2.0 host prepended with http:// for url::Url::parse(), scheme discarded"
    - "K8sManifest split-arm pattern: Deployment gets env traversal, Service does not"
    - "serde default on containers/env Vec fields — valueFrom entries produce value=None, silently skipped"

key_files:
  created: []
  modified:
    - src/plugin/config/openapi.rs
    - src/plugin/config/kubernetes.rs

decisions:
  - "Server.url field is pub url: String on openapiv3::Server (verified via cargo registry grep)"
  - "Swagger 2.0 host extraction uses http:// prefix for URL parsing; emits protocol=http regardless of scheme"
  - "Deployment and Service split into separate match arms so env traversal only applies to Deployment"
  - "valueFrom entries handled implicitly: serde ignores unknown fields, value=None, silently skipped"

metrics:
  duration: "~15 minutes"
  completed: "2026-04-07"
  tasks_completed: 3
  tasks_total: 3
  tests_added: 12
  files_modified: 2
---

# Phase 15 Plan 02: OpenAPI Servers Extraction + Kubernetes Container Env Extraction Summary

**One-liner:** OpenAPI plugin emits ConnectionInfo from servers[].url (OAS3) and host (Swagger 2.0); Kubernetes plugin emits ConnectionInfo from Deployment container env URL-valued keys (DQ-07, DQ-08).

## What Was Implemented

### Task 1: Verify openapiv3 Server.url field

Confirmed via cargo registry grep:
```
/Users/ravichillerega/.cargo/registry/src/.../openapiv3-2.2.0/src/server.rs:
    pub url: String,
```

Field is exactly `pub url: String` on `openapiv3::Server`. The plan assumption was correct — no adaptation needed. `spec.servers` is a `Vec<openapiv3::Server>` and each `.url` is accessed directly.

### Task 2: `src/plugin/config/openapi.rs` (DQ-07)

**ParseResult widened** from 3-tuple to 4-tuple:
```rust
type ParseResult = Result<(Option<ServiceInfo>, Vec<EndpointInfo>, Vec<SchemaInfo>, Vec<ConnectionInfo>), String>;
```

**`parse_openapi_3()`**: Iterates `spec.servers`, calls `parse_url_value(&server.url)`, emits `ConnectionInfo` with `confidence=Medium`, `extraction_method="spec:openapi"`, evidence `"servers[].url = {url}"`. Template URLs (containing `{`) produce `None` from `parse_url_value` and are silently skipped.

**`parse_swagger_2()`**: Added `host: Option<String>` and `base_path: Option<String>` to `SwaggerSpec`. Prepends `http://` to bare hostname for URL parsing, emits connection with `protocol="http"` (Swagger 2.0 is scheme-agnostic).

**`extract()`**: Both match arms destructure 4-tuple and call `result.connections.extend(connections)`.

**Existing tests updated**: 4 tests updated to destructure 4-tuple (mechanical change).

### Task 3: `src/plugin/config/kubernetes.rs` (DQ-08)

**Struct chain added**:
```rust
K8sManifest { spec: Option<K8sSpec> }
K8sSpec { template: Option<K8sTemplate> }
K8sTemplate { spec: Option<K8sPodSpec> }
K8sPodSpec { containers: Vec<K8sContainer> }  // #[serde(default)]
K8sContainer { env: Vec<K8sEnvVar> }          // #[serde(default)]
K8sEnvVar { name: String, value: Option<String> }
```

**Match arm split**: `"Deployment" | "Service"` split into two separate arms. `Deployment` arm runs `is_connection_key` + `parse_url_value` loop after `ServiceInfo` push. `Service` arm pushes `ServiceInfo` only.

**Connection emission**: `source_service = deployment name`, `confidence=High`, `extraction_method="kubernetes"`, evidence `"KEY=value"`.

**valueFrom**: serde ignores `valueFrom` field (no `deny_unknown_fields`), so `value` stays `None` → silently skipped.

## Deviations from Plan

None — plan executed exactly as written.

## Test Count Added

| Module | Tests Added | Total in Module |
|--------|------------|-----------------|
| openapi.rs | 6 (+ 4 updated to 4-tuple) | 12 |
| kubernetes.rs | 6 | 10 |
| **Total new** | **12** | **22** |

All tests passing. `cargo build --release` exits 0 with no new errors.

## Commits

| Hash | Task | Description |
|------|------|-------------|
| `2d90dff` | Task 2 | feat(15-02): extend OpenApiPlugin for server URL extraction (DQ-07) |
| `713266c` | Task 3 | feat(15-02): extend KubernetesPlugin for container env extraction (DQ-08) |

## Self-Check: PASSED

- `src/plugin/config/openapi.rs` modified: FOUND
- `src/plugin/config/kubernetes.rs` modified: FOUND
- Commit `2d90dff` exists: VERIFIED
- Commit `713266c` exists: VERIFIED
- 12 openapi tests pass: VERIFIED
- 10 kubernetes tests pass: VERIFIED
- `cargo build --release` 0 errors: VERIFIED
- `cargo test --all` exits 0: VERIFIED
