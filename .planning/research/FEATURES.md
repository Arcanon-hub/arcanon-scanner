# Feature Landscape

**Domain:** Rust CLI static code analyzer / service dependency scanner
**Project:** Arcanon Scanner
**Researched:** 2026-04-04

---

## Context

Arcanon Scanner occupies an uncommon intersection: it is simultaneously a static analysis CLI and a service dependency mapper. Most comparable tools are either pure SAST (Semgrep, CodeQL, OWASP Noir) or pure service catalog feeders (Backstage entity descriptor generators, architectural drift tools). Arcanon does neither of those — it extracts the structural topology of a codebase (services, endpoints, connections, schemas) for use by a SaaS hub that builds dependency graphs.

The closest analogues are:
- **OWASP Noir** — endpoint detection from source code (Crystal, single binary)
- **Semgrep** — multi-language AST pattern matching CLI
- **Trivy** — scanner-to-hub upload model with structured payload
- **Backstage catalog-import / entity descriptors** — service metadata extraction

This context shapes what is table stakes (learned from all these tools) vs. what is differentiating (unique to this structural-topology use case).

---

## Table Stakes

Features users expect. Missing = product feels incomplete or broken in CI.

| Feature | Why Expected | Complexity | Notes |
|---------|--------------|------------|-------|
| Single binary distribution | Every successful CLI scanner is a single binary. Users will not install runtimes. | Low | Rust compiles to static binary. Already decided. |
| Zero-config first run | `arcanon-scanner .` should produce useful output without any config file. Learned from Semgrep, Trivy, OWASP Noir. | Medium | .arcanon.toml is optional; CLI defaults must be sensible. |
| .gitignore respect | Scanners that traverse node_modules or .git are immediately abandoned. The `ignore` crate handles this. | Low | Built-in excludes plus .gitignore walking. Already spec'd. |
| --dry-run / --output flags | Every CI-integrated tool needs a way to inspect output without side effects. Standard pattern from Snyk, Semgrep, Trivy. | Low | Already in CLI spec. Critical for developer trust. |
| Structured JSON output | Downstream consumers (scripts, CI pipelines, dashboards) require machine-readable output. Expected since 2020. | Low | --output flag writing ScanPayloadV1 to file covers this. |
| Human-readable summary to stdout | Developers scanning locally need to see what was found at a glance. JSON only = hostile UX. | Low | Already spec'd: "print summary (services found, endpoints, connections, upload status)" |
| Meaningful exit codes | CI pipelines gate on exit codes. Exit 0 = success, non-zero = failure. Broken exit codes silently corrupt pipelines. | Low | Must be intentional: 0 = scan OK (upload succeeded or dry-run), 1 = fatal error, 2 = upload failed. |
| --verbose / -v flag | Debugging why a plugin missed something requires log output. Expected by every developer who has used any scanner. | Low | Already in CLI spec (-v, -vv, -vvv). |
| Environment variable support | CI pipelines pass secrets via env vars, not flags. `ARCANON_API_KEY`, `ARCANON_HUB_URL` are non-negotiable. | Low | Already spec'd. |
| Fault-tolerant scanning | A single malformed file or broken plugin should not abort the scan. Users experience this as "the scanner crashed on my repo." | Medium | Already spec'd. Per-file errors are logged and skipped. |
| .gitignore-aware file exclusion | Users expect to control scope. Custom globs in config + CLI flags. | Low | .arcanon.toml `[scanner.exclude]` + `--exclude` flag. |
| Git context detection | Every CI-integrated tool attaches branch/commit to its results. Without this, results cannot be correlated across runs. | Medium | gix-based detection with CI env var fallbacks. Already spec'd. |
| CI pipeline integration documentation | Developers copy-paste CI config. Without a GitHub Actions / GitLab CI snippet, adoption stalls. | Low | Documentation, not code. Critical for enterprise. |
| Retry on transient upload failure | Networks are unreliable in CI. A single 502 should not fail the scan. Observed in every reliable upload-based tool. | Low | 3x exponential backoff already spec'd. |
| Auth via API key | Standard auth pattern for CI-safe scanner upload. No OAuth flow. | Low | ARCANON_API_KEY via header. Already spec'd. |
| Monorepo support | Majority of multi-service codebases today are monorepos. Scanners that flatten everything into one service are useless. | High | Nearest-ancestor file-to-service scoping. Already spec'd. |
| Performance within CI time budgets | < 10s for 1,000 files is the threshold where engineers tolerate a scan in CI. Beyond that, PRs start skipping it. | High | tree-sitter is fast; parallel plugin execution helps. Already spec'd as hard target. |
| Version flag (--version) | Basic hygiene. Required for debugging, issue reports, upgrade tracking. | Low | Already in CLI spec. |
| Makefile / CI build targets | Developers contributing to the scanner need linting, test, and build targets. | Low | Already spec'd: clippy, rustfmt, unit tests. |

---

## Differentiators

Features that set this product apart. Not universally expected, but create real value and competitive separation.

| Feature | Value Proposition | Complexity | Notes |
|---------|-------------------|------------|-------|
| Variable resolution chain | Resolving `.env` → `docker-compose env:` → `k8s ConfigMaps` to dereference connection URLs at scan time. Most scanners emit raw `${DB_HOST}` strings, which the hub cannot interpret. This creates high-fidelity connection data. | High | Unique to service topology tools. OWASP Noir does not do this. |
| Confidence-tagged findings | Tagging each finding as High/Medium/Low confidence lets the hub filter noise and surface uncertain findings for review. Raw scanners emit all findings equally. | Medium | Embedded in type structs. Enables hub-side quality filtering. |
| Evidence snippets on connections | `ConnectionInfo.evidence` carries the code snippet where the outbound call was detected. Enables one-click drill-down from the hub's graph to the source line. | Medium | Differentiates from tools that emit only (source, target, protocol) tuples. |
| Intra-repo connection resolution | Matching outbound calls to exposed endpoints within the same repo at scan time. Gives the hub pre-resolved edges for local service-to-service calls, reducing graph reconciliation work. | High | Hub handles cross-repo; scanner handles intra-repo. Clean separation. |
| Config-driven service overrides | `[services."packages/api"] name = "api-server"` lets teams correct mis-detected service names without modifying code. Critical for repos with unconventional layouts. | Low | Implemented via .arcanon.toml. Enables enterprise adoption. |
| Manual connection declarations | `[[connections.manual]]` blocks let teams annotate runtime-only connections (sidecar proxies, service mesh routes, external SaaS calls) that static analysis cannot detect. | Medium | Honest about static analysis limits; fills gaps pragmatically. |
| Industrial protocol detection | `modbus`, `opcua`, `bacnet` connection protocols. No other general-purpose scanner targets OT/ICS connectivity. | Medium | Free-string protocol field makes this possible without schema churn. |
| Protocol as free string | Avoids the eternal enum debate. Any new protocol (`hl7-fhir`, `nats`, `mqtt5`) works immediately without a scanner release. | Low | Architectural decision already made. Hub must match. |
| 8 config format plugins | OpenAPI, proto, GraphQL, AsyncAPI, docker-compose, Kubernetes, Dockerfile, .env in one pass. Tools like OWASP Noir focus only on endpoint detection. The breadth here is distinctive. | High | Provides structural completeness competitors lack. |
| Framework detection before AST parsing | Checking `package.json` / `go.mod` / `Gemfile` before committing to full AST parsing avoids wasted work and speeds up scans on polyglot repos. | Low | Performance and correctness optimization. |
| Idempotent upload via commit SHA | Re-scanning the same commit is a no-op on the hub. CI retry runs don't produce duplicate graph nodes. This is a data integrity feature most SaaS-connected scanners overlook. | Low | Implemented via `commit_sha` as idempotency key on the hub. |
| Plugin enable/disable at scan time | `--plugins typescript,openapi` lets users scope a scan to what they care about. Enables fast local iteration without full-repo re-scan. | Low | `--plugins` flag already in CLI spec. |
| Source file + line attribution | `ConnectionInfo.source_file` carries `file:line`. Enables direct hyperlinking from hub to code host (GitHub, GitLab). | Low | Already in data model. Differentiates from tools that report findings without line precision. |
| Schema extraction from AST | Typed request/response models extracted from source code, not just from spec files. Fills the gap when teams haven't written OpenAPI specs. | High | Rare in non-security scanners. Enables hub to show schema drift. |

---

## Anti-Features

Features to explicitly NOT build in v1, with rationale.

| Anti-Feature | Why Avoid | What to Do Instead |
|--------------|-----------|-------------------|
| LLM-based analysis | Adds a cloud dependency, an API key, latency, cost, and non-determinism to what should be a fast, offline-capable tool. OWASP Noir added LLM support and users immediately report reliability concerns. | Pure static analysis for v1. LLM is a v2 accuracy enhancer that can be layered on top optionally. |
| External plugin protocol (stdin/stdout JSON) | Adds a subprocess communication protocol, versioning surface, and failure modes. v1 has 15 compiled-in plugins which is sufficient. | Compiled-in plugin registry for v1. External protocol designed but deferred to v2. |
| Incremental scanning / file caching | Requires local state (`.arcanon-cache`) or a hub query for "what did we last scan." Full scans complete in < 10s for 1,000 files, so the ROI is low until scale forces it. | Design the scan loop to be stateless. Revisit when users report > 60s scan times. |
| Vulnerability / CVE detection | That is Snyk, Semgrep, Trivy's job. Mixing security findings into a topology scanner creates a confused product with two different buyer personas. | Refer users to Snyk/Trivy for CVE scanning. Arcanon Scanner is structural only. |
| SARIF output format | SARIF is designed for security findings (CWE IDs, remediation guidance, severity). ScanPayloadV1 is a topology payload, not a findings report. SARIF would be a category error. | ScanPayloadV1 JSON is the output format. Hub renders the graph. |
| Interactive / TUI mode | Adds complexity and creates CI incompatibility risks. Scanners with interactive prompts break piped workflows. | Non-interactive always. Use `--verbose` for feedback. |
| IDE plugin | Separate distribution channel, separate release cycle, separate platform APIs. Dilutes focus. | CI integration is the primary channel. IDE is v3+. |
| macOS / Windows CI builds | Linux covers > 95% of CI runner usage. Multi-platform CI triples the matrix complexity for diminishing return. | Linux amd64 for v1 CI. Binary cross-compiles fine locally on macOS. |
| Homebrew tap / install script | Distribution infrastructure that requires maintenance. Out of scope until v1 validates user demand. | GitHub Releases binary download for v1. |
| Numeric confidence scores (0.0-1.0) | High/Medium/Low is sufficient for v1 hub integration and avoids false precision. The hub's `KNOWN_TOOLS` schema already accepts `ScanPayloadV1` without numeric confidence. | Confidence enum in v1 payload. Numeric extension in v1.1. |
| Auto-fix / code generation | Out of scope for a topology scanner. Generates liability and scope creep. | Read-only analysis only. |
| Recursive cross-repo resolution | The hub is the right place for cross-repo graph reconciliation. Giving the scanner cross-repo awareness creates a circular dependency and requires the scanner to query the hub during scan. | Hub handles cross-repo. Scanner resolves intra-repo only. |
| Daemon / watch mode | Adds process lifecycle management, IPC, and a different operational model. No user signal for this in v1. | Run-and-exit model only. |

---

## Feature Dependencies

```
Git context detection
  └─► ScanPayloadV1 metadata (repo_url, branch, commit_sha)
        └─► Idempotent hub upload

File discovery (ignore crate)
  └─► Plugin dispatch (config + language plugins)
        └─► ExtractionResult (services, endpoints, connections, schemas)
              └─► Variable resolution chain (dereference ${ENV_VAR} in connection URLs)
              └─► Merger (dedup services, aggregate findings)
                    └─► Intra-repo resolver (match outbound → local endpoints)
                          └─► ScanPayloadV1 assembly
                                └─► HTTP upload OR --output / --dry-run

Framework detection heuristic (package.json / go.mod / Gemfile)
  └─► Language plugin AST parsing (tree-sitter queries)
        └─► Endpoint extraction
        └─► Connection extraction (client patterns)
        └─► Schema extraction

Config plugins (OpenAPI, proto, GraphQL, AsyncAPI)
  └─► High-confidence endpoint data (supersedes AST findings)
  └─► Schema data

Config plugins (docker-compose, Kubernetes, Dockerfile)
  └─► Service detection (high-confidence)
  └─► Variable resolution sources (env:, ConfigMaps)

.arcanon.toml service overrides
  └─► Service names / scoping (applied during merger)

.arcanon.toml manual connections
  └─► ConnectionInfo entries added post-merge (before payload assembly)
```

---

## MVP Recommendation

Prioritize (all required for v1 — this is a greenfield with a defined hub contract):

1. **CLI scaffold + config loading** (clap, .arcanon.toml, env vars, --dry-run, --output, exit codes)
2. **Git context + file discovery** (gix, ignore crate, built-in excludes)
3. **Variable resolution chain** (.env, docker-compose, k8s ConfigMaps)
4. **Config plugins** (OpenAPI, proto, docker-compose, Dockerfile) — highest confidence, fastest to implement
5. **Language plugins for TypeScript and Python** — highest adoption, prove out the tree-sitter pattern
6. **Merger + intra-repo resolver + payload assembly** — core correctness logic
7. **HTTP upload with retry** — the integration point with the hub
8. **Remaining language plugins** (Go, Java, C#, Rust, Ruby) — expand coverage
9. **Remaining config plugins** (GraphQL, AsyncAPI, Kubernetes, .env)

Defer to v2:
- External plugin protocol (stdin/stdout JSON) — designed, not built
- LLM enhancement layer — designed, not built
- Incremental scanning — not needed at v1 file counts
- Numeric confidence scores — v1.1 payload extension

---

## Sources

- [OWASP Noir Overview](https://owasp-noir.github.io/noir/get_started/overview/) — endpoint scanner feature reference (HIGH confidence, official docs)
- [OWASP Noir GitHub](https://github.com/owasp-noir/noir) — feature list, output formats, language support (HIGH confidence)
- [CLIG.dev — Command Line Interface Guidelines](https://clig.dev/) — exit codes, output format best practices (HIGH confidence, community standard)
- [Semgrep CLI Reference](https://semgrep.dev/docs/cli-reference) — CI integration patterns, JSON/SARIF output (HIGH confidence, official docs)
- [Sonar — Complete Guide to SARIF](https://www.sonarsource.com/resources/library/sarif/) — why SARIF is security-findings-specific (MEDIUM confidence)
- [Top Static Code Analysis Tools 2026 — DevOpsSchool](https://www.devopsschool.com/blog/top-10-static-code-analysis-tools-in-2025-features-pros-cons-comparison/) — table stakes feature survey (LOW confidence, survey article)
- [Service Dependency Mapping Tools — Virima](https://virima.com/blog/step-by-step-guide-to-service-dependency-mapping-tools) — service map feature expectations (LOW confidence, vendor blog)
- [Incremental SCA Scanning Strategies — Arnica](https://www.arnica.io/blog/incremental-sca-strategies-monorepos) — monorepo + incremental scan patterns (LOW confidence, vendor blog)
- [Structured CLI Output Best Practices — Medium/Metasintaxis](https://medium.com/metasintaxis/structured-cli-output-a-best-practice-for-devops-teams-5d0d6c1d71f5) — stdout/stderr conventions (MEDIUM confidence)
- [Semgrep Multicore Monorepo Blog](https://semgrep.dev/blog/2025/boosting-security-scan-performance-for-monorepos-with-multicore-parallel-processing/) — monorepo performance expectations (MEDIUM confidence, official Semgrep blog)
- [Platform Engineering Anti-Patterns — Jellyfish](https://jellyfish.co/library/platform-engineering/anti-patterns/) — developer friction anti-patterns (LOW confidence)
