---
phase: 09-resolver-and-tech-debt
plan: "03"
subsystem: config
tags: [config, services, tech-debt, toml]
dependency_graph:
  requires: []
  provides: [services-config-parsing]
  affects: [main.rs, ScannerConfig.service_overrides]
tech_stack:
  added: []
  patterns: [TOML table deserialization with dotted keys, HashMap<String, T> serde default]
key_files:
  created: []
  modified:
    - src/config.rs
    - src/main.rs
key_decisions:
  - "ServiceConfig is a separate struct from merger::ServiceOverride — config.rs handles deserialization, main.rs converts to the core type"
  - "Used HashMap<String, ServiceConfig> with #[serde(default)] so missing [services] section gives empty map without error"
metrics:
  duration: "109s"
  completed: "2026-04-05"
  tasks_completed: 2
  tasks_total: 2
  files_modified: 2
---

# Phase 9 Plan 03: [services] Config Parsing Summary

**One-liner:** TOML `[services."path"]` table parsed into `HashMap<String, ServiceConfig>` and converted to `merger::ServiceOverride` in `ScannerConfig`, replacing the placeholder `HashMap::new()` TODO.

## Tasks Completed

| # | Name | Commit | Files |
|---|------|--------|-------|
| 1 | Add ServiceConfig struct and [services] table to ArcanonConfig | 971c65c | src/config.rs |
| 2 | Wire file_cfg.services into ScannerConfig.service_overrides | c916568 | src/main.rs |

## What Was Built

**Task 1 — config.rs:**
- Added `use std::collections::HashMap` import
- Added `ServiceConfig { name: Option<String>, ignore: Option<bool> }` struct after `PatternsConfig`
- Added `pub services: HashMap<String, ServiceConfig>` field to `ArcanonConfig` with `#[serde(default)]`
- Added 5 unit tests covering: name override, ignore flag, missing section (empty map), malformed config (default), both fields set

**Task 2 — main.rs:**
- Replaced `service_overrides: std::collections::HashMap::new(), // TODO: load from .arcanon.toml [services]`
- Now converts `file_cfg.services` via `.into_iter().map(|(k, v)| (k, core::merger::ServiceOverride { name: v.name, ignore: v.ignore })).collect()`
- TODO comment removed — DEBT-02 fully resolved

## Decisions Made

1. `ServiceConfig` kept separate from `merger::ServiceOverride` — deserialization concern lives in `config.rs`, core types live in `core/merger.rs`. Conversion in `main.rs` bridges the two layers.
2. Dotted TOML key syntax (`[services."packages/api"]`) works automatically with `HashMap<String, T>` — no custom deserializer needed.

## Deviations from Plan

None — plan executed exactly as written.

## Known Stubs

None — `services` map is fully wired from config file to `ScannerConfig`. The `apply_service_overrides` function in `merger.rs` already consumes the overrides at scan time.

## Self-Check

Files exist:
- src/config.rs: FOUND
- src/main.rs: FOUND

Commits:
- 971c65c: FOUND
- c916568: FOUND

## Self-Check: PASSED
