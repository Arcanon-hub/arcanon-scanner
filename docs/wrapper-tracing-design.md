# Wrapper Tracing — Design Document

**Created:** 2026-04-05
**Status:** Draft — approved for Phase 7
**Authors:** Ravi, Claude

---

## Problem

Pattern-based detection only catches direct calls to known functions (`fetch`, `axios.get`, `redis.connect`). Real codebases wrap these in custom functions (`apiFetch`, `makeRequest`, `apiClient.get`) and SDK methods (`JournalClient.append`, `EventBus.emit`). The scanner misses all wrapped calls, which are the majority in production code.

## Solution

Two-pass call graph analysis:

**Pass 1 (Build wrapper map):** Scan all function/method definitions. If a function body calls a known connection function → mark it as a wrapper with the underlying protocol.

**Pass 2 (Detect wrapper calls):** Re-scan user code. When a call matches a wrapper in the map → emit a connection with the extracted path/URL from the call arguments.

This works for both in-repo wrappers and library wrappers.

## How It Works

### In-Repo Wrappers

```typescript
// queryClient.ts — Pass 1 finds this
export function apiFetch(path: string) {
    return fetch(`${API_BASE}${path}`, { headers: ... });
}

// useTeams.ts — Pass 2 detects this
const data = await apiFetch(`/api/v1/orgs/${orgId}/teams`);
```

Pass 1: `apiFetch` body contains `fetch(` → wrapper map: `{ "apiFetch": "rest" }`
Pass 2: `apiFetch('/api/v1/orgs/${orgId}/teams')` → ConnectionInfo with path `/api/v1/orgs/{param}/teams`

### Library Wrappers

```python
# site-packages/edgeworks_sdk/transports/http.py — Pass 1 (library scan)
class HTTPTransport:
    def __init__(self, url):
        self.client = httpx.Client(base_url=url)
    
    def append(self, event):
        self.client.post("/v2/append", json=event)

# user code — Pass 2
from edgeworks_sdk import create_client
client = create_client()
client.append(event)
```

Pass 1 (on library source): `HTTPTransport.append` body contains `httpx.post` → wrapper map: `{ "append": "rest" }`
Pass 2 (on user code): `client.append(event)` → ConnectionInfo with protocol "rest"

### Wrapper Chaining

```
fetch()                    → known REST (seed)
apiFetch(path)             → calls fetch → REST wrapper (level 1)
useQuery(path)             → calls apiFetch → REST wrapper (level 2)
```

Each pass iteration extends the wrapper map. Run until no new wrappers found (fixed point).

## Algorithm

```
1. Seed the wrapper map with known connection functions:
   { "fetch": "rest", "axios.get": "rest", "httpx.post": "rest",
     "redis.connect": "redis", "grpc.Dial": "grpc", ... }

2. Pass 1 — Build wrapper map:
   For each function definition in scope (user code + library source):
     Parse function body with tree-sitter
     For each function call in the body:
       If callee is in the wrapper map:
         Add this function to the wrapper map with same protocol
         
   Repeat until wrapper map stops growing (fixed point)

3. Pass 2 — Detect wrapper calls:
   For each file in user code:
     For each function call:
       If callee is in the wrapper map:
         Extract first string argument (path/URL)
         Normalize template literals: ${expr} → {param}
         Emit ConnectionInfo with protocol, path, evidence
```

## Template Literal Extraction

```typescript
apiFetch(`/api/v1/orgs/${orgId}/teams`)
```

1. Detect template literal (backticks or f-string)
2. Replace `${...}` / `{...}` interpolations with `{param}`
3. Result: `/api/v1/orgs/{param}/teams`
4. This matches the hub's path normalization for route matching

Language-specific:
- TypeScript/JavaScript: `` `text ${expr} text` `` → `text {param} text`
- Python: `f"/api/{org_id}/teams"` → `/api/{param}/teams`
- Go: `fmt.Sprintf("/api/%s/teams", orgId)` → `/api/{param}/teams`
- Rust: `format!("/api/{}/teams", org_id)` → `/api/{param}/teams`
- Ruby: `"/api/#{org_id}/teams"` → `/api/{param}/teams`

## Scope

### What gets traced

| Source | Functions found | Example |
|--------|----------------|---------|
| User code (.ts, .py, .go, etc.) | All exported and local functions | `apiFetch`, `makeRequest` |
| Installed libraries (site-packages, node_modules) | Public methods on classes | `JournalClient.append` |
| Lock file deps (Cargo.lock, go.sum) | Not traceable (no source) — fall back to dep tree check (Phase 6) |

### What doesn't get traced

- Dynamic dispatch (`obj[methodName]()`)
- Higher-order functions where the wrapped function is a parameter
- Reflection-based calls
- Code generation output that doesn't exist on disk

## Integration with Existing Phases

| Phase | Role | How wrapper tracing uses it |
|-------|------|---------------------------|
| Phase 4 (Language Plugins) | AST parsing | Tree-sitter queries to find function definitions and call sites |
| Phase 5 (Pattern Engine) | Known functions seed | Pattern registry provides the seed wrapper map |
| Phase 6 (Library Resolution) | Library source discovery | Finds installed package source to scan for library wrappers |

Wrapper tracing sits ON TOP of phases 4-6. It's a new analysis pass, not a replacement.

## Performance

- Pass 1 per file: one tree-sitter parse, extract function definitions + calls (~10ms per file)
- Pass 1 iterations: typically 2-3 until fixed point (most wrappers are 1 level deep)
- Pass 2: same as current scanning but with a larger function set to match
- Total overhead: 1-3 seconds for a 500-file project

Wrapper map cached per-scan — building it once, using it everywhere.

## Data Structures

```rust
/// Maps function/method names to the protocol they wrap
struct WrapperMap {
    /// "apiFetch" → "rest", "client.append" → "rest"
    wrappers: HashMap<String, WrapperInfo>,
}

struct WrapperInfo {
    protocol: String,
    /// Chain: apiFetch → fetch, or append → httpx.post
    chain: Vec<String>,
    /// Where the wrapper was defined
    source: WrapperSource,
}

enum WrapperSource {
    UserCode { file: String, line: usize },
    Library { lib_name: String, file: String },
}
```

## Open Questions

1. **Class method resolution** — `client.append()` matches if `append` is in the wrapper map. But what if multiple classes have an `append` method? Need class-aware matching or accept some false positives.

2. **Scope depth** — How many levels of wrapping to trace? 2 levels covers 99% of real code. Infinite recursion protection needed.

3. **Cross-file tracing** — `apiFetch` defined in `lib/api.ts`, called from `hooks/useTeams.ts`. Need to resolve imports across files. Tree-sitter gives us the AST per file but not cross-file resolution.

4. **Method vs function** — `client.post()` is a method call. `apiFetch()` is a function call. Need both AST patterns.

---

*This document describes Phase 7 of the Arcanon Scanner roadmap.*
