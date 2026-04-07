---
phase: 15
plan: "15-01"
title: "URL Utility + .env Plugin + Compose Plugin Environment Extraction"
subsystem: config-plugins
tags: [url-parsing, env-plugin, compose-plugin, connections, DQ-05, DQ-06]
requirements: [DQ-05, DQ-06]

dependency_graph:
  requires: []
  provides: [url_util, env-connections, compose-env-connections]
  affects: [ExtractionResult.connections]

tech_stack:
  added: []
  patterns:
    - "url::Url::parse() for URL parsing (no hand-rolled split)"
    - "Untagged serde enum for YAML polymorphism (ComposeEnv follows DependsOn pattern)"
    - "Inline .env parsing (no fs::read_to_string — uses file.content from FileContext)"

key_files:
  created:
    - src/plugin/config/url_util.rs
  modified:
    - src/plugin/config/mod.rs
    - src/plugin/config/env.rs
    - src/plugin/config/compose.rs

decisions:
  - "KAFKA_BROKERS handled as special case: split comma list, take first hostname, protocol=kafka"
  - "Map form ComposeEnv only processes string values via .as_str() — skips numbers/booleans"
  - "source_service for env.rs derived from parent dir relative to ctx.root (empty string at root)"

metrics:
  duration: "~20 minutes"
  completed: "2026-04-07T19:43:50Z"
  tasks_completed: 3
  tasks_total: 3
  tests_added: 27
  files_modified: 4
---

# Phase 15 Plan 01: URL Utility + .env Plugin + Compose Plugin Environment Extraction Summary

**One-liner:** Shared URL parsing utility enabling .env and Compose plugins to emit ConnectionInfo from URL-valued environment keys (DQ-05, DQ-06).

## What Was Implemented

### Task 1: `src/plugin/config/url_util.rs` (new file)

Three public functions shared across config plugins:

```rust
pub fn parse_url_value(val: &str) -> Option<(String, String)>
```
Parses a string as a URL. Returns `Some((protocol, hostname))` if valid, not a template variable. Uses `url::Url::parse()` — no hand-rolled splitting.

```rust
pub fn scheme_to_protocol(scheme: &str) -> String
```
Maps URL schemes to canonical protocol strings: `postgres/postgresql` → `"postgresql"`, `redis/rediss` → `"redis"`, `amqp/amqps` → `"amqp"`, `mongodb/mongodb+srv` → `"mongodb"`, `http/https` → `"http"`, `grpc/grpcs` → `"grpc"`, unknown schemes pass through unchanged (free string, no enum per CLAUDE.md).

```rust
pub fn is_connection_key(key: &str) -> bool
```
Returns true for keys ending in `_URL`, `_HOST`, `_ENDPOINT`, `_ADDR`, `_DSN`, or exact matches `DATABASE_URL`, `REDIS_URL`, `AMQP_URL`, `KAFKA_BROKERS`.

Registered in `src/plugin/config/mod.rs` as `pub mod url_util;` before `pub mod openapi;`.

### Task 2: `src/plugin/config/env.rs` (DQ-05)

Replaced marker-only `extract()` with full implementation:
- Parses `.env` content from `file.content` (no `fs::read_to_string`)
- Strips comments, blank lines, `export ` prefix, surrounding quotes
- `KAFKA_BROKERS`: splits on comma, takes first broker, strips port for hostname
- All other connection keys: delegates to `parse_url_value()`
- `source_service`: parent dir of `.env` relative to `ctx.root` (empty at root)
- `extraction_method`: `"spec:env"`, `confidence`: `High`, `dependency`: `None`
- Renamed `test_extract_returns_empty` → `test_extract_skips_non_connection_keys`

### Task 3: `src/plugin/config/compose.rs` (DQ-06)

Extended `ComposeService` with `environment: ComposeEnv` field:
- `ComposeEnv` is an untagged enum: `None | List(Vec<String>) | Map(HashMap<String, serde_yaml_bw::Value>)`
- List form `["KEY=val"]`: split on first `=`
- Map form `{KEY: val}`: extract string values via `.as_str()`, skip non-strings
- All URL-valued connection keys emit `ConnectionInfo` with `source_service` = compose service name
- `extraction_method`: `"compose"`, `confidence`: `High`, `dependency`: `None`
- All 3 original tests still pass (depends_on behavior unchanged)

## Deviations from Plan

None — plan executed exactly as written.

## Test Count Added

| Module | Tests Added | Total in Module |
|--------|------------|-----------------|
| url_util.rs | 13 | 13 |
| env.rs | 8 (7 new + 1 renamed) | 11 |
| compose.rs | 4 | 8 |
| **Total** | **25** | **32** |

All tests passing. `cargo build --release` exits 0 with no new errors.

## Commits

| Hash | Task | Description |
|------|------|-------------|
| `432b0ea` | Task 1 | feat(15-01): add url_util module |
| `3754048` | Task 2 | feat(15-01): enhance EnvPlugin (DQ-05) |
| `b53034a` | Task 3 | feat(15-01): enhance ComposePlugin (DQ-06) |

## Self-Check: PASSED

- `src/plugin/config/url_util.rs` exists: FOUND
- `src/plugin/config/env.rs` modified: FOUND
- `src/plugin/config/compose.rs` modified: FOUND
- Commit `432b0ea` exists: FOUND
- Commit `3754048` exists: FOUND
- Commit `b53034a` exists: FOUND
- 13 url_util tests pass: VERIFIED
- 11 env tests pass: VERIFIED
- 8 compose tests pass: VERIFIED
- `cargo build --release` 0 errors: VERIFIED
