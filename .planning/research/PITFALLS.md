# Domain Pitfalls

**Domain:** Rust CLI multi-language static code analyzer / service dependency scanner
**Researched:** 2026-04-04
**Overall confidence:** HIGH (tree-sitter version pitfalls confirmed via GitHub issues; architectural pitfalls derived from architecture doc analysis + community research)

---

## Critical Pitfalls

Mistakes that cause rewrites, bad scan output, or hard-to-diagnose runtime failures.

---

### Pitfall 1: tree-sitter Grammar / Core Version Mismatch

**What goes wrong:** Each tree-sitter grammar crate (`tree-sitter-typescript`, `tree-sitter-python`, etc.) pins its own version of the `tree-sitter` core crate. When two grammar crates pin different versions, Rust's dependency resolver loads both versions simultaneously. The `Language` type from version `0.24.x` and `Language` from `0.25.x` are considered distinct types by the compiler, producing: `expected tree_sitter::Language, found a different tree_sitter::Language`.

**Why it happens:** Grammar crates use strict version pins (`"~0.24"`) rather than flexible ranges. When the tree-sitter core releases a new minor version, the grammar ecosystem takes weeks-to-months to catch up. You pull in 7 language grammars + 1 config grammar and one of them hasn't updated yet.

**Consequences:** Fails to compile at all. You cannot ship. Happens mid-development when you upgrade one grammar crate and it drags in a newer core.

**Prevention:**
- Pin all grammar crates to the same tree-sitter core version explicitly in `Cargo.toml` using `[patch.crates-io]` or uniform version constraints
- Pick a tree-sitter core version and lock all grammars to that version — do not let Cargo select different versions per grammar
- Add a `cargo tree -d | grep tree-sitter` check to CI to catch duplicate versions before they compound
- Run `cargo update` as a deliberate step, not automatically, so version changes are intentional

**Detection:** `cargo tree --duplicates | grep tree-sitter` shows multiple versions. Compile error with "found a different tree_sitter::Language" message.

**Phase:** Foundation / Phase 1 (setup of Cargo.toml dependencies). Must be right before writing any plugin code.

---

### Pitfall 2: serde_yaml Is Deprecated — Wrong Crate Choice

**What goes wrong:** The architecture specifies `serde_yaml 0.9+`, but `serde-yaml` was deprecated and archived on March 25, 2024, and is no longer maintained. New projects using it will receive no security fixes, have no YAML 1.2 support improvements, and accumulate tech debt from day one.

**Why it happens:** The crate was dominant for years. Architecture docs written before the deprecation (or not updated) still reference it.

**Consequences:** Potential YAML parsing bugs stay unfixed. The crate may be flagged by `cargo audit` in the future. Team wastes time investigating YAML edge case bugs that are already fixed in alternatives.

**Prevention:**
- Use `serde-yml` (a maintained fork of serde_yaml) or `saphyr-serde` as the YAML deserializer
- If `serde_yaml` 0.9 is kept for compatibility, plan a migration to an alternative in the same phase — do not defer it
- Check `cargo audit` in CI to surface deprecated/unmaintained crates

**Detection:** `cargo audit` will flag it. Docs.rs shows "0.9.34+deprecated" in the crate title.

**Phase:** Phase 1 (dependency selection). Fix before any YAML parsing code is written.

---

### Pitfall 3: tree-sitter Queries Break on Prefix-Aggregated Routes (NestJS, Spring Boot, ASP.NET)

**What goes wrong:** Many frameworks split the route path across two annotations: a class-level prefix decorator and a method-level path decorator. A single query matching only `@Get("/path")` on a method will miss the controller prefix entirely, producing partial or wrong endpoint paths.

**Examples of prefix splitting:**
- NestJS: `@Controller("/users")` + `@Get("/:id")` → `/users/:id`
- Spring Boot: `@RequestMapping("/api/v1")` + `@GetMapping("/orders")` → `/api/v1/orders`
- ASP.NET: `[Route("api/[controller]")]` + `[HttpGet("{id}")]` → `/api/users/{id}`

**Why it happens:** AST query language captures individual nodes in the tree. When a path segment lives on the class declaration and another lives on the method, a single query cannot capture both without traversal logic that crosses node boundaries.

**Consequences:** All endpoints for these frameworks are reported with incomplete paths. The intra-repo resolver fails to match them to outbound calls. The hub's cross-repo resolver fails too. The scanner looks broken on any NestJS/Spring/ASP.NET codebase.

**Prevention:**
- For each language plugin that detects prefix-aggregated routes, implement a two-phase extraction: (1) query for class decorators to build a `class_name → prefix` map, (2) query for method decorators and join to the class map
- Write fixture tests for prefix-aggregated patterns in every affected framework before declaring those plugins complete
- Test with real-world code samples, not minimal synthetic fixtures — real codebases almost always use controller prefixes

**Detection:** Tests pass with simple `@Get("/users")` fixtures but fail with `@Controller("/users") + @Get("/:id")` patterns.

**Phase:** Language plugin implementation phases (TypeScript/NestJS, Java/Spring Boot, C#/ASP.NET Core).

---

### Pitfall 4: Mixing Tokio and Rayon Causes Deadlocks

**What goes wrong:** The scanner uses `rayon` for CPU-bound parallel plugin execution and `tokio` for the async HTTP upload. A deadlock occurs if Rayon worker threads call `tokio::block_on()` or if a tokio runtime thread calls blocking Rayon work without using `spawn_blocking`. Tokio's cooperative executor interprets a blocked thread as an unresponsive task and the runtime starves.

**Why it happens:** It's natural to want to add async operations (logging, metrics, file reads) inside plugin `extract()` calls. Plugin code runs on the Rayon thread pool; calling `.await` or `block_on` from there enters the blocked territory.

**Consequences:** The scanner hangs silently after plugins run and before upload. Hard to reproduce — often only shows up on certain host thread counts or specific file counts. `tokio::time::timeout` in upload does not help because the block is at the thread pool level.

**Prevention:**
- Keep a hard boundary: `extract()` on plugins is synchronous and runs entirely on Rayon threads. No `async` methods on `LanguagePlugin`
- The `upload` module is the only async code; it runs in a separate `tokio::main` block in `main.rs`
- If async file I/O is ever needed inside plugins, use `tokio::task::spawn_blocking` to move the Rayon work off the tokio runtime thread, not `block_on`
- Add a clippy lint or comment gate in `plugin/mod.rs` prohibiting `tokio` imports in plugin files

**Detection:** Scanner hangs after "Scanning complete" log line and before upload attempt. `tokio::time::timeout` on upload not triggered — the hang is pre-upload. Thread count in `htop` shows CPU at 0% (stuck threads, not CPU work).

**Phase:** Core engine integration (when parallelism and upload are wired together).

---

### Pitfall 5: Service Merger Creates Duplicate Services from Overlapping Signals

**What goes wrong:** Multiple plugins independently detect the same logical service. A single Node.js microservice may produce four separate `ServiceInfo` entries: one from the `Dockerfile`, one from `docker-compose.yml`, one from `package.json`, and one from the TypeScript language plugin's framework detection. The merger deduplicates by service name, but if the names differ (e.g., `"order-service"` vs `"order_service"` vs `"api"` from the compose file) they survive as separate services.

**Why it happens:** Each detection signal has its own name source. `docker-compose.yml` uses the service block key. `Dockerfile` uses the directory name. `package.json` uses the `"name"` field. These rarely agree exactly in real projects.

**Consequences:** The hub receives 4 services where there is 1. Endpoints get split across duplicate service records. Connection graph becomes inaccurate. Manually overriding via `.arcanon.toml` is the workaround, but users don't know they need to.

**Prevention:**
- The merger must normalize service names (lowercase, replace underscores/spaces with hyphens) before deduplication — do this as a first-class step, not an afterthought
- When multiple signals point to the same `root_path`, merge them into one service regardless of name variation — path proximity is a stronger signal than name equality
- Establish a signal priority for the canonical name: compose key > package.json `"name"` > Dockerfile directory > language plugin inference
- Write merger tests with multi-signal fixture repos that intentionally use different name formats

**Detection:** `--dry-run --output` output shows multiple services with the same `root_path`. Test fixtures for the merger must include multi-signal scenarios.

**Phase:** Merger implementation.

---

## Moderate Pitfalls

Mistakes that cause incorrect output or performance problems but don't require rewrites.

---

### Pitfall 6: tree-sitter Query Explosion on Complex Patterns

**What goes wrong:** tree-sitter's query matching is not always linear in complexity. When patterns use unbounded wildcards or deeply nested alternatives (`(_)` matching many node types), the runtime is proportional to the number of result matches, which can be exponential on large files. A 100KB minified JS file (which should be skipped) or a large generated file that slips past the file guard can take 10–100 seconds per query.

**Prevention:**
- The 500KB file size guard and binary/minified detection guards (in architecture doc section 4) are critical — they must be enforced before any file reaches a tree-sitter query
- Add a per-file timeout for tree-sitter query execution (tree-sitter supports `set_timeout_micros` on the `Parser`). Any file exceeding 5 seconds gets skipped with a logged warning
- Profile queries against real large TypeScript files (monorepos commonly have 5000+ line generated TS files) during development
- Avoid unbounded `(_)*` patterns in queries; prefer specific node types

**Detection:** Wall clock time per file is measurable with `--verbose` tracing. Files causing slowness appear in logs. Check for `gen.ts`, `.d.ts`, or `*.pb.ts` files leaking through guards.

**Phase:** Language plugin development (TypeScript plugin first — JS/TS has the largest and most complex files).

---

### Pitfall 7: Variable Resolution Chain Produces Wrong Connection Targets

**What goes wrong:** The variable resolution chain (.env → compose → k8s ConfigMap) resolves `USER_SERVICE_URL` to a value like `http://user-service:3000`. The scanner extracts `user-service` as the target service name. But in a staging environment `.env.staging`, the same variable resolves to `http://api.external-vendor.com/users`, which is an external service, not a local one. Reading multiple `.env.*` files and merging them (last wins) can produce a target name that matches a real local service by accident.

**Prevention:**
- When the resolved URL is an external hostname (not `localhost`, not a compose service name, not a k8s DNS `*.svc.cluster.local`) → mark confidence as `Low`, not `Medium`
- When multiple `.env` files resolve the same key to different values, emit a warning and use the highest-priority value but log the conflict
- Do not attempt to resolve template string URLs (`` `${BASE_URL}/api/v1/${path}` ``) to actual targets — these should always be `Low` confidence with the raw template as `evidence`, not a guessed target name

**Detection:** Connection targets in `--dry-run` output point to external vendor hostnames or contain interpolation syntax. Manual review of a known codebase reveals external services incorrectly attributed to local services.

**Phase:** Variable resolution implementation and connection detection.

---

### Pitfall 8: YAML Anchors and Multi-Document Files Silently Drop Data

**What goes wrong:** Real-world Kubernetes manifests and docker-compose files use YAML anchors (`&anchor`), aliases (`*alias`), and merge keys (`<<: *defaults`). Many YAML parsers fail silently on these patterns — they either error out, or they expand anchors but lose the aliased values entirely. Multi-document YAML files (separated by `---`) with anchors that reference definitions in a prior document section also break because anchors don't cross the `---` boundary.

**Why it happens:** `serde_yaml` and its successors handle basic anchors, but complex nested merge keys and cross-document patterns hit edge cases. Docker Compose's own parser has had bugs with multiple YAML anchors as recently as 2023.

**Consequences:** Kubernetes ConfigMap values (needed for variable resolution) are silently dropped. Compose `depends_on` connections derived from merged service blocks are missed. No error is reported — the scanner just returns fewer results.

**Prevention:**
- Test the compose and kubernetes plugins against fixtures that use anchors and merge keys
- Log a warning (not an error) when the YAML parser encounters nodes it cannot deserialize — do not silently drop them
- For Kubernetes, test specifically with Helm-generated manifests and multi-document `---`-separated files
- Validate that `VariableStore` contains expected keys after parsing fixture files with anchors

**Detection:** Known ConfigMap values absent from `VariableStore` after parsing. Variable resolution falls through to "unresolved" for keys that should have been found.

**Phase:** Config plugin implementation (compose and kubernetes plugins).

---

### Pitfall 9: Test Fixtures Scan Their Own Source Code

**What goes wrong:** The scanner's test fixture directory (`tests/fixtures/`) contains `.ts`, `.py`, `.go`, etc. files. When running integration tests that invoke the scanner on the repo root (or a parent directory), the scanner may pick up fixture files and include them in results. The fixture for an Express app gets detected as a real service.

**Prevention:**
- Add `tests/fixtures/` to the built-in excludes list, or ensure integration test invocations always specify a narrow `PATH` argument pointing only at the fixture under test
- Unit tests (and the architecture specifies unit tests per plugin with fixtures) should pass fixture content as `FileContext` structs directly, never by scanning the filesystem from the repo root
- CI integration tests that scan real paths must use `--exclude "tests/**"` or `.arcanon.toml` excludes

**Detection:** `--dry-run` output on the scanner's own repo shows services with names like `"express-basic"` from fixture files. Easy to spot but easy to miss if you only run unit tests.

**Phase:** Testing setup (Phase 1) and integration test design.

---

### Pitfall 10: Binary Size Exceeds Target from Unoptimized Grammar Compilation

**What goes wrong:** Each tree-sitter grammar compiles a C parser via `build.rs`. Without aggressive release profile settings, the combined binary of 7 language grammars + 1 core + scanner logic exceeds the 15MB target. Grammar C code is not subject to Rust's dead code elimination unless LTO is enabled. Default release profiles produce 25–40MB binaries for projects with many compiled-C dependencies.

**Prevention:**
- Set the release profile in `Cargo.toml`: `lto = "fat"`, `codegen-units = 1`, `opt-level = "z"`, `strip = "symbols"`
- Use `cargo bloat --release` to identify which grammars and dependencies contribute most to binary size
- The `musl` target for static linking is mentioned in the architecture; verify with `cargo build --target x86_64-unknown-linux-musl --release` + `strip` early in the project, not post-implementation
- Add a CI step that asserts `ls -la target/x86_64-unknown-linux-musl/release/arcanon-scanner | awk '{print $5}' < 15728640` (15MB in bytes)

**Detection:** `ls -lh target/release/arcanon-scanner` > 15MB. Run early. Grows silently as more grammars are added.

**Phase:** Build/CI setup, and validated again after all grammar crates are added.

---

### Pitfall 11: Detached HEAD in CI Produces Empty/Wrong Git Context

**What goes wrong:** GitHub Actions, GitLab CI, and Jenkins all check out in detached HEAD mode by default. `gix` can detect the commit SHA but cannot resolve the branch name from the HEAD ref in detached state. The fallback chain in the architecture (Arcanon env vars → CI provider env vars → `"detached"`) is correct in principle, but if CI pipelines don't set `GITHUB_REF_NAME` or `CI_COMMIT_BRANCH`, the scanner uploads with `branch = "detached"` for every scan. The hub may treat these as different branches and fail to reconcile scan history.

**Prevention:**
- In GitHub Actions, `GITHUB_REF_NAME` is always set for push/PR events — verify this works in the CI job
- For GitLab, `CI_COMMIT_BRANCH` is set on branch pipelines but not on tag pipelines — handle the tag case explicitly
- The deterministic content hash fallback for `commit_sha` (when git is absent) must produce the same hash on the same codebase — verify this doesn't incorporate modification timestamps which would vary between CI runs
- Emit a clear warning to stderr when using the `"detached"` branch fallback so CI users know to set the env var

**Detection:** Hub receives scans with `branch = "detached"` consistently. `--dry-run` output on CI shows `"branch": "detached"`. Add a CI test that asserts branch is not `"detached"`.

**Phase:** Git context implementation and CI setup.

---

## Minor Pitfalls

Annoyances that create misleading output or confuse users but are easy to fix.

---

### Pitfall 12: Rails routes.rb Has Dynamic Route Generation That AST Cannot Detect

**What goes wrong:** Rails `routes.rb` files often use `resources :users` (generates 7 RESTful routes) and `namespace :api` (nests routes under prefix). Both generate multiple routes from a single AST node. A literal query matching `get "/path"` will miss all resourceful routes. Additionally, Rails engines and mounted routes (`mount AdminEngine, at: "/admin"`) are invisible to any static analysis without knowing the engine's route table.

**Prevention:**
- Implement `resources` and `namespace` expansion for the Rails plugin — these are well-documented Rails conventions and can be fully handled with known expansion rules
- Document `mount` and Rails engines as a known limitation in `--verbose` output
- Flag `resources` calls at `Medium` confidence (the paths are known, but Rails may override them with `only:` or `except:` options)

**Detection:** A Rails codebase with standard resourceful routes produces zero endpoints in scan output. Verify against `rails routes` output if available.

**Phase:** Ruby language plugin implementation.

---

### Pitfall 13: HTTP Client Wrapper Functions Produce Missed Connections

**What goes wrong:** Most production codebases wrap HTTP clients in a service layer: `userService.getUser(id)` calls `this.httpClient.get(...)` internally. The scanner detects `this.httpClient.get(USER_SERVICE_URL + "/users/" + id)` but only if it sees the actual HTTP call. If the HTTP call is in a shared library (which is unscoped — no service root ancestor), the connection is dropped with a `source_service = ""` warning.

**Prevention:**
- This is a documented v1 limitation (architecture doc section 13: "connections from unscoped files are dropped")
- Ensure the warning is clearly emitted in `--verbose` output so users can add `.arcanon.toml` service overrides for shared libraries that make HTTP calls
- Consider promoting "connections from shared libraries" to a special reporting category in the payload rather than silently dropping them — emit them with `source_service = null` and let the hub decide

**Detection:** A known outbound HTTP call in `packages/shared/src/api-client.ts` is absent from scan output. Warning appears in verbose log.

**Phase:** Merger implementation and connection aggregation logic.

---

### Pitfall 14: Payload Size Exceeds Hub's 10MB Limit on Large Monorepos

**What goes wrong:** Large monorepos with 50+ services, thousands of endpoints, and many schemas can produce payloads exceeding the 10MB hub limit. The `evidence` field (code snippet attached to each connection) is the primary culprit — a connection with a multi-line template literal evidence snippet, multiplied by thousands of connections, bloats the payload significantly.

**Prevention:**
- Truncate `evidence` strings to 200 characters maximum — enough to show the relevant call, not the entire surrounding function
- Estimate payload size before upload: `serde_json::to_string(&payload)?.len()`. If > 8MB (safety margin), emit a warning and strip `evidence` fields first, then re-measure
- Add a `--dry-run --output /dev/stdout | wc -c` check to the developer workflow documentation
- For v1, the architecture notes < 2MB as typical — monitor this assumption with real-world scans

**Detection:** HTTP 413 response from hub. `--dry-run --output result.json && wc -c result.json` shows > 10MB.

**Phase:** Payload assembly and upload implementation.

---

## Phase-Specific Warnings

| Phase Topic | Likely Pitfall | Mitigation |
|-------------|---------------|------------|
| Dependency setup | Grammar version mismatch (Pitfall 1) | Pin all grammar crates to same tree-sitter core; CI duplicate check |
| Dependency setup | serde_yaml deprecated (Pitfall 2) | Choose serde-yml or saphyr-serde before writing any YAML code |
| Build/CI | Binary size blowout (Pitfall 10) | Set LTO + strip early; add CI size assertion |
| Build/CI | Detached HEAD in CI (Pitfall 11) | Verify `GITHUB_REF_NAME` is set; warn when using fallback |
| YAML config plugins | Anchors/multi-doc YAML (Pitfall 8) | Test with anchor-heavy Helm/compose fixtures |
| Language plugins (TS/Java/C#) | Prefix-aggregated routes (Pitfall 3) | Two-phase extraction per framework; prefix join logic |
| Language plugins (Ruby) | Rails resourceful routes (Pitfall 12) | Expand `resources` macro statically |
| Variable resolution | Wrong target from multi-.env (Pitfall 7) | External hostnames → Low confidence; log conflicts |
| Merger | Duplicate services (Pitfall 5) | Normalize names + merge by root_path, not name alone |
| Core engine wiring | Tokio/Rayon deadlock (Pitfall 4) | Hard sync/async boundary; no tokio imports in plugin code |
| Testing setup | Fixture self-scan (Pitfall 9) | Unit tests pass FileContext structs, not filesystem paths |
| Connection aggregation | Shared library unscoped drops (Pitfall 13) | Emit visible warning; consider null-source connections |
| Payload assembly | Payload too large (Pitfall 14) | Truncate evidence; measure before upload |
| tree-sitter queries | Query explosion on large files (Pitfall 6) | Enforce file guards strictly; add per-file parser timeout |

---

## Sources

- [tree-sitter Versioning Conflict for Grammars' Rust Bindings — GitHub Issue #3095](https://github.com/tree-sitter/tree-sitter/issues/3095) — HIGH confidence (official issue tracker)
- [tree-sitter packaging mess — ABI and version conflicts](https://ayats.org/blog/tree-sitter-packaging) — MEDIUM confidence (community blog, corroborates issue tracker)
- [serde_yaml deprecation — Rust Users Forum](https://users.rust-lang.org/t/serde-yaml-deprecation-alternatives/108868) — HIGH confidence (community discussion + confirmed by docs.rs showing "deprecated" label)
- [serde_yaml 0.9.34+deprecated — docs.rs](https://docs.rs/crate/serde_yaml/latest) — HIGH confidence (official)
- [Knee Deep in tree-sitter Queries — parsiya.net](https://parsiya.net/blog/knee-deep-tree-sitter-queries/) — MEDIUM confidence (practitioner blog with reproducible examples)
- [tree-sitter query performance issue — GitHub Issue #973](https://github.com/tree-sitter/tree-sitter/issues/973) — HIGH confidence (official issue tracker)
- [Mixing rayon and tokio — Lobsters discussion](https://lobste.rs/s/mebxps/mixing_rayon_tokio_for_fun_hair_loss) — MEDIUM confidence (community discussion)
- [Tokio shared state / sync lock in async code — Rust Users Forum](https://users.rust-lang.org/t/potential-deadlock-when-using-sync-lock-in-async-code/121541) — HIGH confidence (official Tokio guidance)
- [Docker Compose multiple YAML anchors bug — Issue #10824](https://github.com/docker/compose/issues/10824) — HIGH confidence (official issue tracker)
- [rust-analyzer VFS circular symlink fix — PR #17093](https://github.com/rust-lang/rust-analyzer/pull/17093) — HIGH confidence (official repo)
- [NestJS Controllers documentation — docs.nestjs.com](https://docs.nestjs.com/controllers) — HIGH confidence (official docs, prefix aggregation pattern confirmed)
- [tree-sitter query complexity — GitHub Discussion #1976](https://github.com/tree-sitter/tree-sitter/discussions/1976) — HIGH confidence (official discussion)
