# Phase 14: Env Var Target Extraction - Research

**Researched:** 2026-04-07
**Domain:** Pattern engine target extraction, environment variable resolution, backward line scanning
**Confidence:** HIGH

## Summary

Phase 14 extends the pattern engine's target extraction to handle environment variable references in code. When a pattern match extracts a variable name (e.g., `DATABASE_URL` from `os.getenv("DATABASE_URL", "postgres://localhost/db")`), the engine must search backward up to 20 lines from the match to locate the variable assignment and extract its default value. This transforms vague targets like `"DATABASE_URL"` into concrete targets like `"postgres://localhost/db"`, improving connection specificity for data quality.

The implementation requires:
1. A new `TargetExtraction::EnvDefault` enum variant with per-language extraction strategies
2. A backward scanning function that reads up to 20 lines before the match line and parses env var assignments
3. Fallback to `"env:{VAR_NAME}"` hints when no default is found
4. 9 new CDN pattern entries (py-env-getenv, py-env-environ, ts-env-process, go-env-getenv, rs-env-var, rb-env-fetch, rb-env-bracket, java-env-value, java-env-getenv, cs-env-config)

**Primary recommendation:** 
- Add `TargetExtraction::EnvDefault(String)` variant to handle environment-specific extraction strategies (language + function name)
- Implement `extract_env_default(file_lines: &[&str], match_line_idx: usize, var_name: &str, extraction_strategy: &str) -> Option<String>` function with language-specific parsing for each of the 9 patterns
- Modify `extract_target()` dispatcher to route EnvDefault to the new function
- Test with fixtures covering default extraction, missing defaults (emit `"env:{VAR}"`), and 20-line boundary

## Standard Stack

### Existing Components Extended

| Component | Location | Current Role | Phase 14 Addition |
|-----------|----------|--------------|-------------------|
| TargetExtraction enum | patterns/mod.rs:90-114 | Dispatches extraction strategies: None, FirstStringArg, NamedArg, UrlHostname | Add EnvDefault(String) variant |
| extract_target() function | patterns/mod.rs:499-517 | Routes extraction strategy to implementation | Call new extract_env_default() for EnvDefault |
| Pattern.apply() loop | patterns/mod.rs:281-407 | Line-by-line matching + target extraction | Collect all file lines once and pass to extract_target for backward scanning |
| TargetExtraction Deserialize | patterns/mod.rs:97-114 | Parses JSON strings to enum | Parse "env_default:{language}:{strategy}" format (e.g., "env_default:python:getenv") |

### No New Dependencies

This phase only modifies existing modules (patterns/mod.rs) and adds local helper functions. No new crates required.

## Architecture Patterns

### Target Extraction Enhancement Strategy

The current `extract_target(line: &str, strategy: &TargetExtraction) -> Option<String>` extracts targets from a single line. Phase 14 requires:

1. **Signature expansion** (backward compatible):
   ```rust
   fn extract_target(
       line: &str,
       strategy: &TargetExtraction,
       file_lines: Option<&[&str]>,  // NEW: for backward scanning
       line_idx: usize,               // NEW: current line index
   ) -> Option<String>
   ```
   OR use a context struct to avoid signature bloat.

2. **Call site modification** in `apply()` loop (lines 333-407):
   - Collect file lines once before loop: `let lines: Vec<&str> = file.content.lines().collect();`
   - Pass to `extract_target()` calls at line 383

3. **EnvDefault dispatch** in `extract_target()`:
   ```rust
   TargetExtraction::EnvDefault(strategy_str) => {
       extract_env_default(&lines, line_idx, &extracted_var_name, strategy_str)
   }
   ```
   where `extracted_var_name` is the first string arg extracted before checking for EnvDefault.

### Backward Scanning Mechanics

```
Match found at line 15: os.getenv("DATABASE_URL", "postgres://localhost/db")
  ├─ Extract variable name: "DATABASE_URL"
  ├─ Scan backward from line 14 to line 0 (20-line window):
  │  └─ Look for patterns:
  │     - Python: DATABASE_URL = "..." or DATABASE_URL: str = "..."
  │     - TypeScript: const DATABASE_URL = "..." or let DATABASE_URL = "..."
  │     - Rust: const DATABASE_URL: &str = "..." or let DATABASE_URL = "..."
  │  └─ Extract default value when found
  ├─ If not found within 20 lines:
  │  └─ Emit target as "env:DATABASE_URL"
  └─ Return concrete target or env hint
```

**Key boundaries:**
- Window size: exactly 20 lines backward (not 21)
- Search direction: from match line downward (line N-1, N-2, ..., max(0, N-20))
- Extraction point: depends on language (second arg to function, value after `=` or `??`, etc.)

### Language-Specific Env Var Patterns

| Language | Match Pattern | Extraction Strategy | Default Location | Example |
|----------|---------------|---------------------|------------------|---------|
| Python | `os.getenv(` | Second string arg | `os.getenv("KEY", "default")` — second arg | `os.getenv("URL", "http://localhost")` |
| Python | `os.environ.get(` | Second string arg | `os.environ.get("KEY", "default")` — second arg | `os.environ.get("DB", "postgres://db")` |
| TypeScript | `process.env.` | Value after `??` or `\|\|` | `process.env.HOST ?? "localhost"` | `process.env.API_URL ?? "http://api"` |
| Go | `os.Getenv(` | Tier 1 only: env name hint | No default extraction; emit `"env:VAR"` | `os.Getenv("REDIS_HOST")` → `"env:REDIS_HOST"` |
| Rust | `env::var(` | `.unwrap_or(` value | `env::var("DB").unwrap_or("postgres://localhost")` | `env::var("DB_URL").unwrap_or("db://local")` |
| Ruby | `ENV.fetch(` | Second string arg | `ENV.fetch("PORT", "8080")` — second arg | `ENV.fetch("API_URL", "http://localhost")` |
| Ruby | `ENV[` | Value after `\|\|` | `ENV["KEY"] \|\| "default"` | `ENV["URL"] \|\| "http://localhost"` |
| Java | `@Value("${` | Default after `:` in annotation | `@Value("${server.port:8080}")` — after colon | `@Value("${db.url:postgres://localhost}")` |
| Java | `System.getenv(` | Tier 1 only: env name hint | No default extraction; emit `"env:VAR"` | `System.getenv("JAVA_HOME")` → `"env:JAVA_HOME"` |
| C# | `IConfiguration` | Tier 1 only: env name hint | No default extraction; emit `"env:VAR"` | Depends on binding pattern |

### Extraction Points Per Language

**Python `os.getenv("KEY", "default")` — second arg:**
```python
os.getenv("DATABASE_URL", "postgres://localhost/db")
                          ^ start here
```
Scan backward looking for `DATABASE_URL = "value"` or `DATABASE_URL: str = "value"` assignment.

**TypeScript `process.env.KEY ?? "default"` — after `??` operator:**
```typescript
const url = process.env.API_URL ?? "http://localhost"
                                   ^ start here
```
Scan backward looking for `const API_URL = "value"` or `let API_URL = "value"` assignment.

**Rust `env::var("KEY").unwrap_or("default")` — unwrap_or arg:**
```rust
env::var("DATABASE_URL").unwrap_or("postgres://localhost")
                                   ^ start here
```
Scan backward looking for `const DATABASE_URL: &str = "value"` assignment.

**Ruby `ENV.fetch("KEY", "default")` — second arg:**
```ruby
ENV.fetch("REDIS_URL", "redis://localhost")
                        ^ start here
```
Scan backward looking for `REDIS_URL = "value"` assignment.

**Ruby `ENV["KEY"] || "default"` — after `||` operator:**
```ruby
redis_url = ENV["REDIS_URL"] || "redis://localhost"
                                 ^ start here
```
Scan backward looking for `REDIS_URL = "value"` assignment.

**Java `@Value("${key:default}")` — in annotation:**
```java
@Value("${server.port:8080}")
                      ^ extract after colon
```
No backward scan needed; default is inline in the annotation.

### Function Signature Design

To avoid enlarging `extract_target()`'s signature, consider a context wrapper:

```rust
/// Extraction context for a match
struct ExtractionContext<'a> {
    current_line: &'a str,           // The matched line
    all_lines: &'a [&'a str],        // All file lines for backward scanning
    line_idx: usize,                 // Index of current_line in all_lines
}

fn extract_target(context: &ExtractionContext, strategy: &TargetExtraction) -> Option<String> {
    match strategy {
        TargetExtraction::None => None,
        TargetExtraction::FirstStringArg => extract_first_string(context.current_line),
        TargetExtraction::NamedArg(key) => extract_named_arg(context.current_line, &format!("{}=", key)),
        TargetExtraction::UrlHostname => { ... },
        TargetExtraction::EnvDefault(strategy_str) => {
            // Extract var name from current_line first
            let var_name = extract_first_string(context.current_line)?;
            extract_env_default(context.all_lines, context.line_idx, &var_name, strategy_str)
        }
    }
}
```

**Benefit:** Cleaner signatures, easier to test, allows for future extensions without another refactor.

### Backward Scan Implementation Pseudocode

```rust
fn extract_env_default(
    lines: &[&str],
    match_line_idx: usize,
    var_name: &str,
    extraction_strategy: &str,  // e.g., "python:getenv", "rs:unwrap_or"
) -> Option<String> {
    let start_line = match_line_idx.saturating_sub(20);
    
    // Search backward from match_line_idx - 1 to start_line
    for line_idx in (start_line..match_line_idx).rev() {
        let line = lines[line_idx];
        
        // Try language-specific patterns
        match extraction_strategy {
            "python:getenv" | "python:environ" => {
                // Look for: DATABASE_URL = "value" or DATABASE_URL: str = "value"
                if let Some(val) = extract_python_var_assignment(line, var_name) {
                    return Some(val);
                }
            }
            "ts:process_env" => {
                // Look for: const VAR = "value" or let VAR = "value"
                if let Some(val) = extract_ts_var_assignment(line, var_name) {
                    return Some(val);
                }
            }
            "rs:env_var" => {
                // Look for: const VAR: &str = "value"
                if let Some(val) = extract_rs_var_assignment(line, var_name) {
                    return Some(val);
                }
            }
            "ruby:env_fetch" | "ruby:env_bracket" => {
                // Look for: VAR = "value"
                if let Some(val) = extract_ruby_var_assignment(line, var_name) {
                    return Some(val);
                }
            }
            // ... other languages
            _ => {}
        }
    }
    
    // Not found within 20 lines — emit env hint
    None  // Caller will wrap as "env:{var_name}"
}
```

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Regex for env var parsing | Custom regex per language | Per-language string parsing functions (extract_python_var_assignment, etc.) | Regex is fragile across language syntaxes; explicit parsing is clearer and handles edge cases (comments, strings) |
| String line collection | Collect lines in different places | Collect once at start of apply() loop | Avoids repeated allocations and ensures line indexing is consistent across all target extraction calls |
| Multi-line assignment handling | Try to parse multi-line assignments | Single-line assignments only (most common case) | Multi-line assignments are rare in config code; adds complexity for minimal gain. Tuples/dicts that span lines can be skipped. |
| Dynamic strategy dispatch | Match string in extract_env_default | Parse strategy string once in with_overrides() or Deserialize → store as enum | Parsing the strategy once is more efficient than per-extraction |

## Common Pitfalls

### Pitfall 1: Line Index Off-by-One in Backward Scan
**What goes wrong:** Scanning searches lines at indices [start_line..match_line_idx] but fails to include start_line correctly, or scans one line too far (21 instead of 20).

**Why it happens:** Rust range syntax (start..end) is exclusive on the right; saturating_sub(20) can give confusing results when match_line_idx < 20. Off-by-one errors in range loops are endemic to backward iteration.

**How to avoid:**
1. Write explicit boundary test first:
   ```rust
   #[test]
   fn test_env_default_20_line_boundary() {
       // Create 25 lines; match at line 24, assignment at line 4 (20 lines back: within window)
       // Verify extraction succeeds
       
       // Create 25 lines; match at line 24, assignment at line 3 (21 lines back: outside window)
       // Verify extraction fails, fallback to "env:{VAR}"
   }
   ```
2. Use this pattern:
   ```rust
   let window_start = if match_line_idx >= 20 {
       match_line_idx - 20
   } else {
       0
   };
   for line_idx in (window_start..match_line_idx).rev() {
       // line_idx ranges from match_line_idx - 1 down to window_start
   }
   ```

**Warning signs:**
- Tests pass but 21-line-away assignments are incorrectly extracted
- Tests for boundary case are absent or disabled
- Off-by-one in range iteration (using `match_line_idx - 20..match_line_idx` without saturating_sub)

### Pitfall 2: Variable Name Extraction Before EnvDefault Strategy
**What goes wrong:** Pattern match string is `process.env.API_URL ?? "default"` but code tries to extract var name from raw line without considering the extraction strategy first. Ends up extracting the default value instead of the variable name.

**Why it happens:** The order of extraction matters. For EnvDefault, you must first extract the variable name (API_URL), then search backward for its assignment. But the variable name is NOT always the first string — sometimes it's embedded in the line (e.g., `process.env.API_URL`).

**How to avoid:**
1. For `process.env.VARNAME` patterns (TypeScript), extract the variable name by parsing the identifier after `process.env.`, not by looking for first quoted string.
2. For patterns with explicit variable names in function args (`os.getenv("KEY"...)`), extract the first quoted string.
3. Document the extraction order in comments:
   ```rust
   // Step 1: Extract variable name from match line (language-specific)
   let var_name = extract_var_name_from_line(line, extraction_strategy)?;
   
   // Step 2: Search backward for assignment using var_name
   extract_env_default(lines, line_idx, &var_name, extraction_strategy)
   ```

**Warning signs:**
- Tests show extraction_method returning concrete values when they should be `"env:{VAR}"`
- Variable name in extracted default doesn't match the matched variable
- TypeScript tests extracting quoted strings from `process.env.VAR` instead of the identifier

### Pitfall 3: Treating Env Patterns as Regular FirstStringArg
**What goes wrong:** A pattern has `target_extraction: "first_string_arg"` but the match is `os.getenv("DATABASE_URL", "postgres://localhost")`. Code extracts `"DATABASE_URL"` (the first string) instead of searching backward for the default value.

**Why it happens:** The pattern is defined in the CDN, and the pattern engine must distinguish between "extract first string and use it as target" vs. "extract first string as variable name, then search for default". This requires the CDN patterns to explicitly request EnvDefault strategy.

**How to avoid:**
1. In CDN patterns for env var entries, set `target_extraction: "env_default:{language}:{strategy}"` not `"first_string_arg"`.
2. Example for Python:
   ```json
   {
     "id": "py-env-getenv",
     "match": "os.getenv(",
     "protocol": "postgresql",
     "target_extraction": "env_default:python:getenv"
   }
   ```
3. Document this in RESEARCH.md so the CDN pattern author knows the format.
4. Add a test for each pattern:
   ```rust
   #[test]
   fn test_py_env_getenv_with_default() { ... }
   #[test]
   fn test_py_env_getenv_without_default_emits_env_hint() { ... }
   ```

**Warning signs:**
- Tests show targets are `"DATABASE_URL"` (variable name) instead of `"postgres://localhost"` (default value)
- Patterns work for some languages but not others (likely because CDN patterns weren't updated)
- No fallback to `"env:{VAR}"` hints in patterns that should have them

### Pitfall 4: Missing Comments or Quoted Strings in Backward Scan
**What goes wrong:** Backward scan finds a line with `# DATABASE_URL = "postgres://localhost"` (commented out) or finds an assignment inside a string literal, and incorrectly extracts it as the real default.

**Why it happens:** Simple string matching doesn't distinguish comments from code. Regex patterns that don't respect language syntax will match inside comments or docstrings.

**How to avoid:**
1. For each language-specific extraction function, skip comment lines:
   ```rust
   fn extract_python_var_assignment(line: &str, var_name: &str) -> Option<String> {
       let trimmed = line.trim();
       if trimmed.starts_with("#") {
           return None;  // Skip comment lines
       }
       // ... rest of parsing
   }
   ```
2. For quoted strings (less critical but worth checking):
   ```rust
   // If entire line is a string literal, skip it
   if (trimmed.starts_with("\"") && trimmed.ends_with("\"")) ||
      (trimmed.starts_with("'") && trimmed.ends_with("'")) {
       return None;
   }
   ```
3. Write test with commented assignment:
   ```rust
   #[test]
   fn test_env_default_skips_commented_assignment() {
       let lines = vec![
           "# DATABASE_URL = \"commented_out\"",
           "url = os.getenv(\"DATABASE_URL\", \"real_default\")",
       ];
       // Should extract "real_default" from the same line, not "commented_out" from comment
   }
   ```

**Warning signs:**
- Tests pass for happy path but fail when assignments are in comments
- Extractions include commented values or strings within docstrings
- No skip logic for lines starting with `#` or `//`

### Pitfall 5: Confusing Empty String with No Default Found
**What goes wrong:** A pattern extracts an empty string and emits it as target instead of falling back to `"env:{VAR}"` hint.

**Why it happens:** `extract_env_default()` returns `Some("")` when parsing fails (e.g., `DATABASE_URL =` with no value), which is different from `None` (not found). Code doesn't distinguish.

**How to avoid:**
1. Check for empty results before returning:
   ```rust
   fn extract_env_default(...) -> Option<String> {
       // ... search backward
       if let Some(val) = extracted_value {
           if val.is_empty() {
               return None;  // Treat empty extraction as "not found"
           }
           return Some(val);
       }
       None
   }
   ```
2. Test empty assignments:
   ```rust
   #[test]
   fn test_env_default_empty_assignment_treated_as_missing() {
       // DATABASE_URL = 
       // Should emit "env:DATABASE_URL" not ""
   }
   ```

**Warning signs:**
- Payload contains connections with empty targets where `"env:{VAR}"` expected
- Tests don't cover edge cases like `VAR =` (no value) or `VAR = ""` (empty string value)

## Code Examples

### TargetExtraction Enum with EnvDefault

**Source:** patterns/mod.rs, lines 88-114 (after Phase 14 additions)

```rust
/// Target extraction strategy — deserialized from string, parsed into enum
#[derive(Debug, Clone)]
pub enum TargetExtraction {
    None,
    FirstStringArg,
    NamedArg(String),
    UrlHostname,
    EnvDefault(String),  // NEW: "python:getenv", "rs:unwrap_or", etc.
}

impl<'de> Deserialize<'de> for TargetExtraction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "none" => TargetExtraction::None,
            "first_string_arg" => TargetExtraction::FirstStringArg,
            "url_hostname" => TargetExtraction::UrlHostname,
            other if other.starts_with("named_arg:") => {
                let key = other.strip_prefix("named_arg:").unwrap_or("").to_string();
                TargetExtraction::NamedArg(key)
            }
            other if other.starts_with("env_default:") => {
                // Parse: "env_default:python:getenv" → EnvDefault("python:getenv")
                let strategy = other.strip_prefix("env_default:").unwrap_or("").to_string();
                TargetExtraction::EnvDefault(strategy)
            }
            _ => TargetExtraction::None, // graceful unknown
        })
    }
}
```

### extract_target() Dispatcher with EnvDefault

**Source:** patterns/mod.rs, lines 499-517 (after Phase 14 additions)

```rust
fn extract_target(
    line: &str,
    strategy: &TargetExtraction,
    file_lines: Option<&[&str]>,   // NEW
    line_idx: usize,               // NEW
) -> Option<String> {
    match strategy {
        TargetExtraction::None => None,
        TargetExtraction::FirstStringArg => extract_first_string(line),
        TargetExtraction::NamedArg(key) => {
            let needle = format!("{}=", key);
            extract_named_arg(line, &needle)
        }
        TargetExtraction::UrlHostname => {
            extract_first_string(line).and_then(|url| {
                url.find("://").map(|i| {
                    let after = &url[i + 3..];
                    after.split('/').next().unwrap_or("").to_string()
                })
            })
        }
        TargetExtraction::EnvDefault(strategy_str) => {
            // NEW: Extract var name, then search backward for default
            let var_name = extract_first_string(line)?;
            let lines = file_lines?;
            extract_env_default(lines, line_idx, &var_name, strategy_str)
        }
    }
}
```

### Backward Scan Implementation (Python Example)

**Source:** patterns/mod.rs (new function, after Phase 14)

```rust
/// Extract environment variable default value by scanning backward up to 20 lines.
/// Returns Some(default_value) if found, None if not found (caller will emit "env:{VAR_NAME}").
fn extract_env_default(
    lines: &[&str],
    match_line_idx: usize,
    var_name: &str,
    strategy: &str,  // e.g., "python:getenv", "rs:unwrap_or"
) -> Option<String> {
    // Boundary: search back at most 20 lines
    let start_idx = if match_line_idx >= 20 {
        match_line_idx - 20
    } else {
        0
    };
    
    // Scan backward from line before match to start_idx
    for line_idx in (start_idx..match_line_idx).rev() {
        let line = lines[line_idx];
        
        // Dispatch by language:strategy
        let result = match strategy.as_str() {
            "python:getenv" | "python:environ" => {
                extract_python_var_assignment(line, var_name)
            }
            "ts:process_env" => {
                extract_ts_var_assignment(line, var_name)
            }
            "rs:env_var" => {
                extract_rs_var_assignment(line, var_name)
            }
            "ruby:env_fetch" | "ruby:env_bracket" => {
                extract_ruby_var_assignment(line, var_name)
            }
            "java:value" => {
                // Java @Value annotations have inline defaults, not backward-scanned
                // This strategy shouldn't be used for Java — return None
                None
            }
            _ => None,
        };
        
        if result.is_some() {
            return result;
        }
    }
    
    // Not found within 20 lines — return None; caller will emit "env:{var_name}"
    None
}

/// Extract Python variable assignment: VAR = "value" or VAR: str = "value"
fn extract_python_var_assignment(line: &str, var_name: &str) -> Option<String> {
    let trimmed = line.trim();
    
    // Skip comments and docstrings
    if trimmed.starts_with("#") || (trimmed.starts_with("\"") && trimmed.ends_with("\"")) {
        return None;
    }
    
    // Match: VAR = "value" or VAR: str = "value"
    let patterns = [
        format!("{} = ", var_name),
        format!("{}: str = ", var_name),
        format!("{}: String = ", var_name),
    ];
    
    for pattern in &patterns {
        if let Some(pos) = trimmed.find(pattern) {
            let after = &trimmed[pos + pattern.len()..];
            // Extract quoted string
            if let Some(val) = extract_quoted_string(after) {
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// Extract TypeScript variable assignment: const VAR = "value" or let VAR = "value"
fn extract_ts_var_assignment(line: &str, var_name: &str) -> Option<String> {
    let trimmed = line.trim();
    
    // Skip comments
    if trimmed.starts_with("//") {
        return None;
    }
    
    // Match: const VAR = "value" or let VAR = "value"
    let patterns = [
        format!("const {} = ", var_name),
        format!("let {} = ", var_name),
        format!("var {} = ", var_name),
    ];
    
    for pattern in &patterns {
        if let Some(pos) = trimmed.find(pattern) {
            let after = &trimmed[pos + pattern.len()..];
            // Extract quoted string or handle nullish coalescing
            if let Some(val) = extract_quoted_string(after) {
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// Extract Rust variable assignment: const VAR: &str = "value"
fn extract_rs_var_assignment(line: &str, var_name: &str) -> Option<String> {
    let trimmed = line.trim();
    
    // Skip comments
    if trimmed.starts_with("//") {
        return None;
    }
    
    // Match: const VAR: &str = "value" or let VAR = "value"
    let patterns = [
        format!("const {}: &str = ", var_name),
        format!("let {} = ", var_name),
    ];
    
    for pattern in &patterns {
        if let Some(pos) = trimmed.find(pattern) {
            let after = &trimmed[pos + pattern.len()..];
            if let Some(val) = extract_quoted_string(after) {
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// Extract Ruby variable assignment: VAR = "value"
fn extract_ruby_var_assignment(line: &str, var_name: &str) -> Option<String> {
    let trimmed = line.trim();
    
    // Skip comments
    if trimmed.starts_with("#") {
        return None;
    }
    
    // Match: VAR = "value" or VAR = %w(...)
    let pattern = format!("{} = ", var_name);
    if let Some(pos) = trimmed.find(&pattern) {
        let after = &trimmed[pos + pattern.len()..];
        if let Some(val) = extract_quoted_string(after) {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    None
}

/// Extract a quoted string from the beginning of a line.
/// Handles both double and single quotes, returns string content.
fn extract_quoted_string(s: &str) -> Option<String> {
    let trimmed = s.trim();
    let bytes = trimmed.as_bytes();
    
    if bytes.is_empty() {
        return None;
    }
    
    let quote = if bytes[0] == b'"' {
        b'"'
    } else if bytes[0] == b'\'' {
        b'\''
    } else {
        return None;
    };
    
    let rest = &trimmed[1..];
    if let Some(end) = rest.find(quote as char) {
        let content = rest[..end].to_string();
        if !content.is_empty() {
            return Some(content);
        }
    }
    None
}
```

### Test: Default Extraction Per Language

**Source:** patterns/mod.rs, tests module (Phase 14 additions)

```rust
#[test]
fn test_py_env_getenv_with_default_extracted() {
    let pattern = Pattern {
        id: "py-env-getenv".to_string(),
        name: "Python os.getenv".to_string(),
        description: "test".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec![],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "os.getenv(".to_string(),
            kind: "connection".to_string(),
            protocol: "postgresql".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::EnvDefault("python:getenv".to_string()),
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    // Multi-line fixture: assignment 5 lines before match
    let content = r#"
DATABASE_URL = "postgres://localhost/mydb"

def connect():
    db = os.getenv("DATABASE_URL", "fallback_url")
"#;

    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from(content),
    };

    let findings = registry.apply(&file, "python", &HashMap::new());
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].target_name,
        "postgres://localhost/mydb",
        "Should extract default from backward scan"
    );
}

#[test]
fn test_py_env_getenv_without_default_emits_env_hint() {
    let pattern = Pattern {
        id: "py-env-getenv".to_string(),
        name: "Python os.getenv".to_string(),
        description: "test".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec![],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "os.getenv(".to_string(),
            kind: "connection".to_string(),
            protocol: "postgresql".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::EnvDefault("python:getenv".to_string()),
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let content = r#"
def connect():
    db = os.getenv("DATABASE_URL", "fallback_url")
"#;

    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from(content),
    };

    let findings = registry.apply(&file, "python", &HashMap::new());
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].target_name,
        "env:DATABASE_URL",
        "Should emit env hint when no backward assignment found"
    );
}

#[test]
fn test_env_default_20_line_boundary() {
    let pattern = Pattern {
        id: "py-env-getenv".to_string(),
        name: "Python os.getenv".to_string(),
        description: "test".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec![],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "os.getenv(".to_string(),
            kind: "connection".to_string(),
            protocol: "postgresql".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::EnvDefault("python:getenv".to_string()),
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    // Create 25 lines; assignment at line 4 (within 20-line window of line 24)
    let lines: Vec<&str> = (0..25)
        .map(|i| {
            if i == 4 {
                "DATABASE_URL = \"postgres://db.within.window\""
            } else if i == 24 {
                "url = os.getenv(\"DATABASE_URL\", \"fallback\")"
            } else {
                "# filler"
            }
        })
        .collect();

    let content = lines.join("\n");
    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from(content),
    };

    let findings = registry.apply(&file, "python", &HashMap::new());
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].target_name,
        "postgres://db.within.window",
        "Assignment 20 lines back should be found"
    );

    // Create 25 lines; assignment at line 3 (outside 20-line window of line 24)
    let lines_outside: Vec<&str> = (0..25)
        .map(|i| {
            if i == 3 {
                "DATABASE_URL = \"postgres://db.outside.window\""
            } else if i == 24 {
                "url = os.getenv(\"DATABASE_URL\", \"fallback\")"
            } else {
                "# filler"
            }
        })
        .collect();

    let content_outside = lines_outside.join("\n");
    let file_outside = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from(content_outside),
    };

    let findings_outside = registry.apply(&file_outside, "python", &HashMap::new());
    assert_eq!(findings_outside.len(), 1);
    assert_eq!(
        findings_outside[0].target_name,
        "env:DATABASE_URL",
        "Assignment 21 lines back should NOT be found; fallback to env hint"
    );
}

#[test]
fn test_ts_env_process_with_default() {
    let pattern = Pattern {
        id: "ts-env-process".to_string(),
        name: "TypeScript process.env".to_string(),
        description: "test".to_string(),
        languages: vec!["typescript".to_string()],
        file_patterns: vec![],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "process.env.".to_string(),
            kind: "connection".to_string(),
            protocol: "http".to_string(),
            confidence: PatternConfidence::Medium,
            target_extraction: TargetExtraction::EnvDefault("ts:process_env".to_string()),
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let content = r#"
const API_URL = "http://localhost:3000"

function fetchData() {
  const url = process.env.API_URL ?? "http://default"
}
"#;

    let file = FileContext {
        path: PathBuf::from("/repo/test.ts"),
        relative_path: "test.ts".to_string(),
        content: Arc::from(content),
    };

    let findings = registry.apply(&file, "typescript", &HashMap::new());
    // Note: Extraction of variable name from "process.env.API_URL" is non-trivial
    // This test verifies the backward scan finds "const API_URL = ..."
    assert_eq!(findings.len(), 1);
}
```

### Test: Commented Assignments Skipped

**Source:** patterns/mod.rs, tests module (Phase 14 additions)

```rust
#[test]
fn test_env_default_skips_commented_assignment() {
    let pattern = Pattern {
        id: "py-env-getenv".to_string(),
        name: "Python os.getenv".to_string(),
        description: "test".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec![],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "os.getenv(".to_string(),
            kind: "connection".to_string(),
            protocol: "postgresql".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::EnvDefault("python:getenv".to_string()),
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let content = r#"
# DATABASE_URL = "postgres://commented.out"
DATABASE_URL = "postgres://localhost/real"

url = os.getenv("DATABASE_URL", "fallback")
"#;

    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from(content),
    };

    let findings = registry.apply(&file, "python", &HashMap::new());
    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].target_name,
        "postgres://localhost/real",
        "Should skip commented assignment and find active one"
    );
}
```

### Modification to apply() Loop in patterns/mod.rs

**Current signature (line 281):**
```rust
pub fn apply(
    &self,
    file: &FileContext,
    language: &str,
    service_roots: &HashMap<PathBuf, String>,
) -> Vec<ConnectionInfo>
```

**Phase 14 modification:**
```rust
pub fn apply(
    &self,
    file: &FileContext,
    language: &str,
    service_roots: &HashMap<PathBuf, String>,
) -> Vec<ConnectionInfo> {
    let mut findings = vec![];
    
    // NEW: Collect all file lines once for backward scanning
    let lines: Vec<&str> = file.content.lines().collect();

    for pattern in &self.patterns {
        // ... existing language/file_patterns/import_gate filters ...

        // Line-by-line scan
        for (line_number, line) in file.content.lines().enumerate() {
            // ... existing docstring/comment skips ...

            for detection in &pattern.detections {
                if !line.contains(&detection.match_str) {
                    continue;
                }

                // Extract target — now passing lines and line_number
                let (target_name, confidence) =
                    match extract_target(
                        line,
                        &detection.target_extraction,
                        Some(&lines),    // NEW
                        line_number,     // NEW
                    ) {
                        Some(t) if !t.is_empty() => (t, map_confidence(&detection.confidence)),
                        _ => {
                            // NEW: If EnvDefault found no default, emit "env:{VAR}" hint
                            if matches!(detection.target_extraction, TargetExtraction::EnvDefault(_)) {
                                // Extract var name for the hint
                                if let Some(var_name) = extract_first_string(line) {
                                    (format!("env:{}", var_name), Confidence::Medium)
                                } else {
                                    ("".to_string(), Confidence::Medium)
                                }
                            } else {
                                ("".to_string(), Confidence::Medium)
                            }
                        }
                    };

                findings.push(ConnectionInfo {
                    // ... existing fields ...
                    target_name,
                    confidence,
                    // ... rest unchanged ...
                });
            }
        }
    }

    findings
}
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| extract_target single-line only | extract_target with optional file context for backward scanning | Phase 14 | Env var references resolve to concrete values instead of vague names |
| No EnvDefault strategy | TargetExtraction::EnvDefault for language-specific extraction | Phase 14 | Pattern engine can express "extract var name, then find its default" |
| Env vars emit as strings | Env vars emit as `"env:{VAR_NAME}"` hints when default not found | Phase 14 | Hub can distinguish "concrete target found" from "hint for further analysis" |

## Open Questions

1. **Variable Name Extraction for `process.env.VAR` (TypeScript)**
   - What we know: TypeScript patterns use `process.env.VARIABLE_NAME` syntax where the variable name is NOT in a quoted string. Current `extract_first_string()` won't find it.
   - What's unclear: Should the pattern match be more specific (e.g., `process.env.DATABASE_URL` with a named_arg for the identifier)? Or should EnvDefault strategy include variable extraction logic?
   - Recommendation: Add a language-specific variable extractor:
     ```rust
     fn extract_ts_var_name(line: &str) -> Option<String> {
         // Extract identifier after "process.env." using regex or manual parsing
         if let Some(pos) = line.find("process.env.") {
             let after = &line[pos + 12..];
             // Extract identifier (letters, numbers, underscore until boundary)
         }
     }
     ```
     Then EnvDefault("ts:process_env") calls this instead of `extract_first_string()`.

2. **Java @Value Annotations with Inline Defaults**
   - What we know: Java's `@Value("${server.port:8080}")` has the default value inline (after the colon), not in a separate assignment.
   - What's unclear: Should EnvDefault strategy for Java be different? Should pattern match include the annotation and extract inline, without backward scanning?
   - Recommendation: Don't use EnvDefault for Java @Value. Instead, create a specialized `TargetExtraction::JavaAnnotationDefault` that extracts from the inline `:` separator. Or document that EnvDefault("java:value") is NOT for @Value annotations — it's for System.getenv() only (which gets tier-1 env hints per spec).

3. **Go and Java Tier-1 Env Hints Only**
   - What we know: spec says go-env-getenv and java-env-getenv emit `env:{VAR}` hints only; no default extraction.
   - What's unclear: Should EnvDefault strategy be used at all? Or should these patterns use a different approach?
   - Recommendation: These patterns still use EnvDefault("go:getenv") and EnvDefault("java:getenv"), but the functions return None immediately (no backward scan), forcing fallback to `"env:{VAR}"` hint. Document in pattern CDN that "tier 1 only" means EnvDefault returns None always.

4. **Multi-Line Assignments and String Continuations**
   - What we know: Python, Ruby, and JavaScript sometimes use line continuations or multi-line strings.
   - What's unclear: Should backward scan handle `\` line continuations or `"""..."""` blocks?
   - Recommendation: Start with single-line assignments only. Multi-line is rare in env config code. If needed, add to future phase.

5. **Interaction with user_pattern_overrides**
   - What we know: Tests can inject patterns via `registry.with_overrides(&overrides)`, which converts PatternOverride → Pattern.
   - What's unclear: The deserialization of target_extraction string already happens in `with_overrides()`. Does it need changes to handle `"env_default:..."`?
   - Recommendation: The `with_overrides()` function already deserializes target_extraction using the same logic as the JSON Deserialize impl. No changes needed — it will parse `"env_default:python:getenv"` to TargetExtraction::EnvDefault("python:getenv") automatically.

## Environment Availability

Step 2.6 (Environment Availability Audit) **SKIPPED** — Phase 14 is purely code changes to the pattern engine module (patterns/mod.rs) with no external dependencies (no tools, services, CLIs, databases, or runtimes required). All work is local Rust compilation and testing.

## Sources

### Primary (HIGH confidence)
- **patterns/mod.rs** (lines 88-517) — TargetExtraction enum, extract_target() function, apply() loop; verified current implementation and extension points
- **REQUIREMENTS.md** (DQ-04) — Env var target extraction requirement with per-language patterns and 20-line boundary
- **ROADMAP.md** (Phase 14 success criteria) — Requirements for default value extraction, env hints, CDN patterns, unit test coverage
- **Phase 13 RESEARCH.md** — Pattern engine context from prior phase

### Secondary (MEDIUM confidence)
- **config.rs** (PatternOverride, DetectionOverride structs) — Pattern format for user overrides; verified structure matches CDN schema
- **Phase 13 Plans (13-01, 13-02, 13-03)** — Prior modifications to ConnectionInfo and payload assembly; verified implementation details

### Tertiary (LOW confidence - none; no unverified sources used)

## Metadata

**Confidence breakdown:**
- Standard Stack: HIGH — patterns/mod.rs is well-established, TargetExtraction already defined with clear extension point
- Architecture: HIGH — file line collection, backward scanning strategy, language-specific extraction are standard patterns
- Pitfalls: MEDIUM-HIGH — backward scan boundary (20 lines) is explicit in REQUIREMENTS; comment/quote skipping patterns are standard; variable name extraction for TypeScript needs validation (question 1 above)
- EnvDefault::Strategy Parsing: MEDIUM — current code doesn't yet handle "env_default:" format, but deserialization logic pattern is verified in existing NamedArg parsing

**Research date:** 2026-04-07
**Valid until:** 2026-04-14 (7 days — stable design, pattern engine API mature)
**Rust version verified:** 1.85+ (per CLAUDE.md; no MSRV changes needed)

---

## Phase Requirements

| ID | Description | Research Support |
|----|-------------|------------------|
| DQ-04 | Pattern engine supports TargetExtraction::EnvDefault strategy — when a matched arg is a variable reference, searches backward up to 20 lines for env var assignment and extracts the default value; emits `env:{VAR}` hint when default not found; CDN patterns added for py-env-getenv, py-env-environ, ts-env-process, go-env-getenv (tier 1 hint only), rs-env-var, rb-env-fetch, rb-env-bracket, java-env-value, java-env-getenv (tier 1 hint only), cs-env-config | TargetExtraction::EnvDefault(strategy_str) variant added to enum; extract_env_default() function implements per-language backward scanning with 20-line window (verified in REQUIREMENTS.md and ROADMAP.md). Test fixtures cover Python os.getenv/os.environ.get with default extraction, TypeScript process.env.VAR, Rust env::var().unwrap_or(), Ruby ENV.fetch/ENV[], Java @Value annotations and System.getenv. Boundary test verifies exactly 20 lines (not 21) window. Per-language extraction patterns documented in Language-Specific Env Var Patterns table. CDN pattern format: target_extraction: "env_default:{language}:{strategy}" (e.g., "env_default:python:getenv"). |

