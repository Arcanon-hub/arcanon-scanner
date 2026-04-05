# Phase 7: Wrapper Tracing - Context

**Gathered:** 2026-04-05
**Status:** Ready for planning
**Source:** Design doc + discussion

<domain>
## Phase Boundary

Two-pass call graph analysis that discovers function wrappers around known connection functions — in user code AND in installed libraries — then detects calls to those wrappers with path/URL extraction including template literal normalization.

</domain>

<decisions>
## Implementation Decisions

### Algorithm
- **D-01:** Two-pass approach: Pass 1 builds wrapper map from function definitions, Pass 2 detects wrapper calls in user code.
- **D-02:** Seed the wrapper map with known connection functions from the CDN pattern registry (fetch, axios.get, httpx.post, redis.connect, etc.).
- **D-03:** Iterate Pass 1 until fixed point (wrapper map stops growing). Typically 2-3 iterations. Cap at 5 to prevent infinite loops.
- **D-04:** Wrapper map is a HashMap<String, WrapperInfo> mapping function/method name → protocol + chain.

### In-Repo Wrapper Detection
- **D-05:** Use tree-sitter to parse function definitions. For each function, check if its body contains a call to something in the wrapper map.
- **D-06:** Handle both standalone functions (`function apiFetch()`) and class methods (`class Client { async append() {} }`).
- **D-07:** Cross-file resolution: if `apiFetch` is defined in `lib/api.ts` and called from `hooks/useTeams.ts`, both are in the same scan scope — the wrapper map is global across all files in the scan.

### Library Wrapper Detection
- **D-08:** Reuse Phase 6 library source discovery (venv, node_modules). Run Pass 1 on library source to find library method wrappers.
- **D-09:** Library wrappers go into the same wrapper map as in-repo wrappers. No separate handling needed.

### Template Literal Extraction
- **D-10:** Replace interpolated expressions with `{param}`: `${expr}` → `{param}`, `{var}` → `{param}`, `%s` → `{param}`, `#{}` → `{param}`.
- **D-11:** This matches the hub's existing path normalization (resolver already does `:param` → `{param}`).

### Scope Limits
- **D-12:** Max wrapper chain depth: 5 levels. Beyond that, likely false positive.
- **D-13:** Skip functions with more than 200 lines (too complex, likely not a simple wrapper).
- **D-14:** For class methods, match by method name only (not class-qualified). Accept some false positives from name collisions — better than missing connections.

### Claude's Discretion
- Module structure (new module src/wrapper/ vs extension of existing)
- Tree-sitter query design for function definition extraction per language
- How to extract function body call sites efficiently
- Integration point in scanner.rs (before or after current pattern engine)

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Design Document
- `docs/wrapper-tracing-design.md` — Full algorithm, template literal handling, data structures, open questions

### Scanner Integration Points
- `src/core/scanner.rs` — Language map loop where wrapper tracing plugs in
- `src/patterns/mod.rs` — PatternRegistry provides the seed wrapper map (known connection functions)
- `src/libres/mod.rs` — Library source discovery (venv, node_modules paths)
- `src/ast/mod.rs` — AstHelper for tree-sitter queries

### Real-World Test Case
- `/Users/ravichillerega/sources/arcanon-hub/packages/dashboard/src/lib/queryClient.ts` — `apiFetch` wrapper around `fetch`
- `/Users/ravichillerega/sources/arcanon-hub/packages/dashboard/src/hooks/useTeams.ts` — Calls `apiFetch('/api/v1/orgs/${orgId}/teams')`

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- `AstHelper::query_matches()` — runs tree-sitter queries, returns capture matches. Reuse for function definition extraction.
- `PatternRegistry.patterns` — known connection functions, becomes the seed wrapper map.
- `LibraryResolver` — finds installed library source paths. Reuse for library wrapper scanning.
- `walk_repo()` — walks directories for file discovery.

### Established Patterns
- Tree-sitter queries per language already exist in language plugins (route detection). Similar queries needed for function definition extraction.
- Scanner language_map loop already iterates per language. Wrapper tracing runs in the same loop.

### Integration Points
- Wrapper tracing runs AFTER pattern engine and library resolution in scanner.rs
- Results feed into the same `pattern_results` vec before merger

</code_context>

<specifics>
## Specific Ideas

- The arcanon-hub dashboard is the primary validation target — `apiFetch` wrapper detection + template literal path extraction
- The edgeworks-sdk is the library wrapper validation target — `HTTPTransport.append` → `httpx.post`

</specifics>

<deferred>
## Deferred Ideas

- Dynamic dispatch tracing (`obj[methodName]()`)
- Higher-order function wrapping (function passed as parameter)
- Cross-repo wrapper resolution (scanner only sees one repo at a time)
- Runtime tracing integration (complementary to static analysis)

</deferred>

---

*Phase: 07-wrapper-tracing*
*Context gathered: 2026-04-05*
