//! Phase 7: Two-pass wrapper tracing.
//!
//! Wrapper tracing discovers function wrappers around known connection functions,
//! enabling the scanner to detect calls to these wrappers and extract their paths/URLs
//! with template literal normalization.
//!
//! # Two-Pass Algorithm
//!
//! **Pass 1:** Build the wrapper map by scanning function definitions across all files
//! (user code + libraries). If a function body calls a known connection function,
//! mark it as a wrapper and add it to the map.
//!
//! **Pass 2:** Re-scan user code to detect calls to functions in the wrapper map,
//! extract their path/URL arguments, and emit ConnectionInfo results.

use crate::ast::AstHelper;
use crate::patterns::PatternRegistry;
use crate::plugin::FileContext;
use crate::types::{Confidence, ConnectionInfo, ExtractionResult};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::debug;

/// Maps function/method names to the protocol they wrap (D-04).
/// Global across all files in a scan (D-07).
#[derive(Debug, Clone)]
pub struct WrapperMap {
    wrappers: HashMap<String, WrapperInfo>,
}

/// Information about a detected wrapper function.
#[derive(Debug, Clone)]
pub struct WrapperInfo {
    /// Protocol the wrapper ultimately connects to ("rest", "grpc", "redis", etc.)
    pub protocol: String,
    /// Call chain: ["apiFetch", "fetch"] — innermost last
    pub chain: Vec<String>,
    /// Where the wrapper was defined
    #[allow(dead_code)]
    pub source: WrapperSource,
    /// Depth in the wrapper chain (D-12: max 5)
    pub depth: usize,
}

/// Where a wrapper function was found.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum WrapperSource {
    UserCode { file: String, line: usize },
    Library { lib_name: String, file: String },
}

impl WrapperMap {
    pub fn new() -> Self {
        Self {
            wrappers: HashMap::new(),
        }
    }

    pub fn insert(&mut self, name: String, info: WrapperInfo) {
        self.wrappers.insert(name, info);
    }

    pub fn contains(&self, name: &str) -> bool {
        self.wrappers.contains_key(name)
    }

    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Option<&WrapperInfo> {
        self.wrappers.get(name)
    }

    pub fn len(&self) -> usize {
        self.wrappers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.wrappers.is_empty()
    }

    /// Iterate over all (name, info) pairs
    pub fn iter(&self) -> impl Iterator<Item = (&String, &WrapperInfo)> {
        self.wrappers.iter()
    }
}

impl Default for WrapperMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize template literals and format strings to use {param} placeholders (D-10, D-11).
///
/// Handles:
/// - TypeScript/JS: `${expr}` → `{param}`, backtick stripping
/// - Python f-strings: `{expr}` inside f"..." → `{param}`
/// - Go: `%s`, `%d`, `%v` (fmt.Sprintf style) → `{param}`
/// - Rust: `{}` (format! style) → `{param}`
/// - Ruby: `#{expr}` → `{param}`
pub fn normalize_template_literal(raw: &str) -> String {
    // Strip outer backtick quotes (TypeScript template literals)
    let s = raw.trim_matches('`');

    // Strip outer f"..." Python f-string delimiters
    let s = if (s.starts_with("f\"") && s.ends_with('"'))
        || (s.starts_with("f'") && s.ends_with('\''))
    {
        &s[2..s.len() - 1]
    } else {
        s
    };

    // Strip outer single/double quotes
    let s = s.trim_matches('"').trim_matches('\'');

    let mut result = s.to_string();

    // TypeScript/JS: ${...} → {param}
    // Use a simple state-machine approach for nested braces
    result = replace_pattern_braced(&result, "${", '}');

    // Ruby: #{...} → {param}
    result = replace_pattern_braced(&result, "#{", '}');

    // Python f-string: {expr} → {param} (only if expr contains non-whitespace)
    // Match {identifier} patterns (not {param} already replaced)
    result = replace_python_fstring_vars(&result);

    // Go/C: %s, %d, %v, %f, %g, %x, %q → {param}
    result = replace_go_fmt(&result);

    // Rust format!: {} or {name} → {param}
    result = replace_rust_format(&result);

    result
}

/// Replace pattern like "${...}" or "#{...}" with "{param}".
/// Scans for opening pattern, finds matching closing char (tracking brace depth),
/// replaces entire span with "{param}".
fn replace_pattern_braced(s: &str, open: &str, close: char) -> String {
    let mut result = String::new();
    let mut remaining = s;

    while !remaining.is_empty() {
        if remaining.starts_with(open) {
            // Skip the open pattern
            remaining = &remaining[open.len()..];

            // Find the matching close, tracking nested braces
            let mut depth = 1;
            let mut found = false;
            for (i, ch) in remaining.chars().enumerate() {
                if ch == '{' {
                    depth += 1;
                } else if ch == close && depth == 1 {
                    // Found the matching close
                    remaining = &remaining[i + 1..];
                    result.push_str("{param}");
                    found = true;
                    break;
                } else if ch == close {
                    depth -= 1;
                }
            }

            if !found {
                // No matching close found, push the {param} anyway
                result.push_str("{param}");
                remaining = "";
            }
        } else {
            let ch = remaining.chars().next().unwrap();
            result.push(ch);
            remaining = &remaining[ch.len_utf8()..];
        }
    }

    result
}

/// Replace Python f-string variables: {identifier} → {param}
/// Matches {identifier} where identifier is [a-zA-Z_][a-zA-Z0-9_.]*
fn replace_python_fstring_vars(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            // Look ahead to check if this is an identifier or {param}
            let mut is_identifier = false;

            if let Some(&next_ch) = chars.peek() {
                if next_ch.is_alphabetic() || next_ch == '_' {
                    is_identifier = true;
                }
            }

            if is_identifier {
                // Consume identifier
                while let Some(&next_ch) = chars.peek() {
                    if next_ch.is_alphanumeric() || next_ch == '_' || next_ch == '.' {
                        chars.next();
                    } else {
                        break;
                    }
                }

                // Check for closing }
                if let Some(&'}') = chars.peek() {
                    chars.next();
                    result.push_str("{param}");
                } else {
                    // Not a valid identifier pattern, restore
                    result.push('{');
                }
            } else {
                result.push('{');
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Replace Go/C format specifiers: %s, %d, %v, %f, %g, %x, %q → {param}
fn replace_go_fmt(s: &str) -> String {
    let mut result = s.to_string();

    for specifier in &['s', 'd', 'v', 'f', 'g', 'x', 'q'] {
        result = result.replace(&format!("%{}", specifier), "{param}");
    }

    result
}

/// Replace Rust format! style: {} or {identifier} → {param}
fn replace_rust_format(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '{' {
            if let Some(&next_ch) = chars.peek() {
                if next_ch == '}' {
                    // {} → {param}
                    chars.next();
                    result.push_str("{param}");
                } else if next_ch.is_alphabetic() || next_ch == '_' {
                    // {identifier} → {param}
                    while let Some(&inner_ch) = chars.peek() {
                        if inner_ch == '}' {
                            chars.next();
                            result.push_str("{param}");
                            break;
                        } else if inner_ch.is_alphanumeric() || inner_ch == '_' {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                } else {
                    result.push('{');
                }
            } else {
                result.push('{');
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Build the initial wrapper map by seeding from pattern registry known functions (D-02).
/// For each pattern detection, strip trailing "(" from match_str to get function name.
/// E.g., "fetch(" → "fetch", "axios.get(" → "axios.get", "Redis(" → "Redis"
fn seed_from_patterns(
    registry: &PatternRegistry,
    language: &str,
    files: &[FileContext],
) -> WrapperMap {
    let mut map = WrapperMap::new();

    // Pre-compute: which import gates are actually present in the scanned files?
    // This prevents seeding from patterns whose libraries aren't used in this codebase.
    let all_content: String = files
        .iter()
        .map(|f| f.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n");

    for pattern in registry.patterns() {
        // Only seed from patterns matching the current language
        if !pattern.languages.is_empty() && !pattern.languages.iter().any(|l| l == language) {
            continue;
        }

        // Check import gate: at least one gate string must appear in the scanned files.
        // This prevents opcua patterns from seeding when no file imports opcua.
        if !pattern.import_gate.is_empty() {
            let gate_present = pattern
                .import_gate
                .iter()
                .any(|gate| all_content.contains(gate));
            if !gate_present {
                continue;
            }
        }

        for detection in &pattern.detections {
            // Strip trailing "(" to get bare function name
            let name = detection.match_str.trim_end_matches('(').to_string();
            // Skip very short names that cause false positives (e.g., "Client", ".get")
            if name.len() < 4 || name.starts_with('.') {
                continue;
            }
            if !name.is_empty() && !map.contains(&name) {
                map.insert(
                    name.clone(),
                    WrapperInfo {
                        protocol: detection.protocol.clone(),
                        chain: vec![name.clone()],
                        source: WrapperSource::UserCode {
                            file: "seed".to_string(),
                            line: 0,
                        },
                        depth: 0,
                    },
                );
            }
        }
    }
    map
}

/// Count the number of lines in a string.
fn count_lines(content: &str) -> usize {
    content.lines().count()
}

/// Scan a single file and return new wrapper entries found.
/// Returns Vec<(name, WrapperInfo)> of new wrappers found.
fn extract_function_wrappers_from_file(
    file: &FileContext,
    current_map: &WrapperMap,
    is_library: bool,
    lib_name: &str,
) -> Vec<(String, WrapperInfo)> {
    let mut found = Vec::new();
    let content = file.content.as_ref();

    // Language detection from file extension
    let ext = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // Use tree-sitter-based approach for TypeScript/JavaScript, Python
    // line-based for others
    match ext {
        "ts" | "tsx" | "js" | "jsx" => {
            extract_ts_wrappers(content, file, current_map, is_library, lib_name, &mut found);
        }
        "py" => {
            extract_py_wrappers(content, file, current_map, is_library, lib_name, &mut found);
        }
        _ => {
            // For Go, Rust, Java, C#, Ruby: use line-based heuristic
            extract_line_based_wrappers(
                content,
                file,
                current_map,
                is_library,
                lib_name,
                &mut found,
            );
        }
    }

    found
}

/// Extract wrappers from TypeScript/JavaScript files using tree-sitter.
fn extract_ts_wrappers(
    content: &str,
    file: &FileContext,
    current_map: &WrapperMap,
    is_library: bool,
    lib_name: &str,
    found: &mut Vec<(String, WrapperInfo)>,
) {
    let language: tree_sitter::Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
    let helper = AstHelper::new(language);

    // Query for function declarations and arrow functions
    let query = r#"
(function_declaration
  name: (identifier) @fn_name
  body: (statement_block) @fn_body)
(method_definition
  name: (property_identifier) @fn_name
  body: (statement_block) @fn_body)
(variable_declarator
  name: (identifier) @fn_name
  value: (arrow_function) @fn_body)
"#;

    let matches = helper.query_matches(content, query);

    // Extract function name and body pairs from matches
    let mut fn_data: HashMap<String, String> = HashMap::new();
    for m in &matches {
        if m.capture_name == "fn_name" {
            fn_data.insert(m.node_text.clone(), String::new());
        } else if m.capture_name == "fn_body" {
            // For now, just collect it. We'll pair them in a second pass.
        }
    }

    // Simple approach: look for function definitions line by line and extract bodies
    for line_pair in matches.chunks(2) {
        if line_pair.len() == 2 && line_pair[0].capture_name == "fn_name" {
            let fn_name = &line_pair[0].node_text;
            let fn_body = &line_pair[1].node_text;

            check_function_and_add_to_wrapper_map(
                fn_name,
                fn_body,
                file,
                current_map,
                is_library,
                lib_name,
                found,
            );
        }
    }
}

/// Extract wrappers from Python files using tree-sitter.
fn extract_py_wrappers(
    content: &str,
    file: &FileContext,
    current_map: &WrapperMap,
    is_library: bool,
    lib_name: &str,
    found: &mut Vec<(String, WrapperInfo)>,
) {
    let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    let helper = AstHelper::new(language);

    // Query for function definitions
    let query = r#"
(function_definition
  name: (identifier) @fn_name
  body: (block) @fn_body)
"#;

    let matches = helper.query_matches(content, query);

    // Extract function name and body pairs from matches
    for line_pair in matches.chunks(2) {
        if line_pair.len() == 2 && line_pair[0].capture_name == "fn_name" {
            let fn_name = &line_pair[0].node_text;
            let fn_body = &line_pair[1].node_text;

            check_function_and_add_to_wrapper_map(
                fn_name,
                fn_body,
                file,
                current_map,
                is_library,
                lib_name,
                found,
            );
        }
    }
}

/// Extract wrappers from other language files (Go, Rust, Java, C#, Ruby) using line-based heuristic.
fn extract_line_based_wrappers(
    content: &str,
    file: &FileContext,
    current_map: &WrapperMap,
    is_library: bool,
    lib_name: &str,
    found: &mut Vec<(String, WrapperInfo)>,
) {
    let ext = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let lines: Vec<&str> = content.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        // Detect function definition based on language
        let fn_name: Option<String> = match ext {
            "go" => {
                // Go: func name(
                if line.contains("func ") {
                    line.split("func ")
                        .nth(1)
                        .and_then(|rest| rest.split('(').next())
                        .map(|s| s.trim().to_string())
                } else {
                    None
                }
            }
            "rs" => {
                // Rust: fn name(
                if line.contains("fn ") {
                    line.split("fn ")
                        .nth(1)
                        .and_then(|rest| rest.split('(').next())
                        .map(|s| s.trim().to_string())
                } else {
                    None
                }
            }
            "java" => {
                // Java: (public|private|protected|static) ... name(
                let re_patterns = ["pub ", "private ", "protected ", "static "];
                let has_pattern = re_patterns.iter().any(|p| line.contains(p));
                if has_pattern {
                    line.split('(')
                        .next()
                        .and_then(|before| before.split_whitespace().last())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            }
            "rb" => {
                // Ruby: def name
                if line.contains("def ") {
                    line.split("def ")
                        .nth(1)
                        .and_then(|rest| rest.split('(').next())
                        .map(|s| s.trim().to_string())
                } else {
                    None
                }
            }
            "cs" => {
                // C#: (public|private|protected) ... name(
                let re_patterns = ["public ", "private ", "protected "];
                let has_pattern = re_patterns.iter().any(|p| line.contains(p));
                if has_pattern {
                    line.split('(')
                        .next()
                        .and_then(|before| before.split_whitespace().last())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(name) = fn_name {
            // Collect function body until closing brace
            let mut fn_body = String::new();
            let mut brace_depth = 0;
            let mut in_function = false;
            let mut line_count = 0;

            for line_content in &lines[i..] {
                line_count += 1;
                fn_body.push_str(line_content);
                fn_body.push('\n');

                for ch in line_content.chars() {
                    if ch == '{' {
                        in_function = true;
                        brace_depth += 1;
                    } else if ch == '}' {
                        brace_depth -= 1;
                        if in_function && brace_depth == 0 {
                            // End of function
                            check_function_and_add_to_wrapper_map(
                                &name,
                                &fn_body,
                                file,
                                current_map,
                                is_library,
                                lib_name,
                                found,
                            );
                            break;
                        }
                    }
                }
                if in_function && brace_depth == 0 && line_count > 1 {
                    break;
                }
            }
        }
    }
}

/// Generic lifecycle/utility function names that must never be treated as wrappers (WRAP-09).
/// These names are too common and ambiguous — treating them as wrappers produces false positives.
const WRAPPER_BLOCKLIST: &[&str] = &[
    "__init__",
    "run",
    "main",
    "start",
    "stop",
    "setup",
    "teardown",
    "clear",
    "close",
    "shutdown",
    "cleanup",
    "reset",
    "init",
    "dispose",
    "destroy",
    "configure",
    "register",
];

/// Check if a function calls something in the wrapper map and add it if so.
fn check_function_and_add_to_wrapper_map(
    fn_name: &str,
    fn_body: &str,
    file: &FileContext,
    current_map: &WrapperMap,
    is_library: bool,
    lib_name: &str,
    found: &mut Vec<(String, WrapperInfo)>,
) {
    // Skip functions with >200 lines (D-13)
    let line_count = count_lines(fn_body);
    if line_count > 200 {
        debug!("Skipping {}: body has {} lines (>200)", fn_name, line_count);
        return;
    }

    // Skip generic lifecycle/utility function names that are never real wrappers (WRAP-09)
    if WRAPPER_BLOCKLIST.contains(&fn_name) {
        debug!(
            "Skipping {}: name is on wrapper blocklist (WRAP-09)",
            fn_name
        );
        return;
    }

    // For each entry in current_map, check if fn_body calls it
    for (callee_name, callee_info) in current_map.iter() {
        // Check if fn_body contains callee_name followed by (
        if fn_body.contains(&format!("{}(", callee_name)) {
            // Found a call to something in the wrapper map
            let new_depth = callee_info.depth + 1;

            // Skip if depth > 2 (D-12, WRAP-08: real wrappers are 1-2 hops)
            if new_depth > 2 {
                debug!(
                    "Skipping {}: wrapper chain depth {} exceeds max (2)",
                    fn_name, new_depth
                );
                return;
            }

            // Build new chain: [fn_name, ...callee_info.chain]
            let mut new_chain = vec![fn_name.to_string()];
            new_chain.extend(callee_info.chain.clone());

            let source = if is_library {
                WrapperSource::Library {
                    lib_name: lib_name.to_string(),
                    file: file.relative_path.clone(),
                }
            } else {
                WrapperSource::UserCode {
                    file: file.relative_path.clone(),
                    line: 1, // Simplified — could parse actual line number
                }
            };

            found.push((
                fn_name.to_string(),
                WrapperInfo {
                    protocol: callee_info.protocol.clone(),
                    chain: new_chain,
                    source,
                    depth: new_depth,
                },
            ));

            // Only match the first callee per function
            break;
        }
    }
}

/// Pass 1: Build the wrapper map from user code and library files.
/// Seeds from pattern registry, then iterates to fixed point (D-03, max 3 iterations (depth 2 converges faster)).
///
/// # Arguments
/// * `user_files` - All files from the user's codebase
/// * `lib_files` - Library source files (from LibraryResolver, if any)
/// * `registry` - PatternRegistry providing seed functions
///
/// # Returns
/// WrapperMap shared across all files in the scan (D-06, D-07)
pub fn build_wrapper_map(
    user_files: &[FileContext],
    lib_files: &[(String, Vec<FileContext>)], // (lib_name, files)
    registry: &PatternRegistry,
    language: &str,
) -> WrapperMap {
    let mut map = seed_from_patterns(registry, language, user_files);
    let initial_count = map.len();
    debug!("Wrapper map seeded with {} known functions", initial_count);

    // Fixed-point iteration: repeat until map stops growing or max 3 iterations (D-03)
    for iteration in 0..3 {
        let prev_len = map.len();

        // Scan user files
        for file in user_files {
            let new_entries = extract_function_wrappers_from_file(file, &map, false, "");
            for (name, info) in new_entries {
                if !map.contains(&name) {
                    debug!(
                        "Pass 1 iter {}: found wrapper {} → {} ({})",
                        iteration,
                        name,
                        info.chain.last().unwrap_or(&"?".to_string()),
                        info.protocol
                    );
                    map.insert(name, info);
                }
            }
        }

        // Scan library files (D-08, D-09)
        for (lib_name, files) in lib_files {
            for file in files {
                let new_entries = extract_function_wrappers_from_file(file, &map, true, lib_name);
                for (name, info) in new_entries {
                    if !map.contains(&name) {
                        debug!(
                            "Pass 1 iter {}: found library wrapper {} → {} ({})",
                            iteration,
                            name,
                            info.chain.last().unwrap_or(&"?".to_string()),
                            info.protocol
                        );
                        map.insert(name, info);
                    }
                }
            }
        }

        let new_len = map.len();
        debug!(
            "Pass 1 iteration {}: {} → {} wrappers",
            iteration, prev_len, new_len
        );
        if new_len == prev_len {
            debug!(
                "Wrapper map reached fixed point after {} iterations",
                iteration + 1
            );
            break;
        }
    }

    debug!(
        "Wrapper map complete: {} total wrappers ({} from seed, {} discovered)",
        map.len(),
        initial_count,
        map.len() - initial_count
    );
    map
}

/// Extract first string/template-literal argument from a function call line.
/// Returns the normalized string (template literals → {param}).
/// Returns None if no string literal or template literal is found in args.
fn extract_string_arg_from_call(line: &str, callee: &str) -> Option<String> {
    // Find the call site: callee + "("
    let call_pattern = format!("{callee}(");
    let call_pos = line.find(&call_pattern)?;
    let after_open = &line[call_pos + call_pattern.len()..];

    // Check for template literal argument (backtick)
    if let Some(bt_start) = after_open.find('`') {
        let rest = &after_open[bt_start..];
        if let Some(bt_end) = rest[1..].find('`') {
            let raw = &rest[..bt_end + 2]; // include both backticks
            return Some(normalize_template_literal(raw));
        }
    }

    // Check for f-string (Python)
    if let Some(f_pos) = after_open.find("f\"").or_else(|| after_open.find("f'")) {
        let quote_char = after_open.chars().nth(f_pos + 1).unwrap_or('"');
        let rest = &after_open[f_pos..];
        let end_quote = if quote_char == '"' { '"' } else { '\'' };
        if let Some(end) = rest[2..].find(end_quote) {
            let raw = &rest[..end + 3];
            return Some(normalize_template_literal(raw));
        }
    }

    // Check for regular string literal (" or ')
    let bytes = after_open.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'"' || b == b'\'' {
            let quote = b as char;
            let rest = &after_open[i + 1..];
            if let Some(end) = rest.find(quote) {
                let raw = rest[..end].to_string();
                // Normalize in case it contains format specifiers
                return Some(normalize_template_literal(&raw));
            }
        }
    }

    None
}

/// Pass 2: Scan files for calls to wrappers in the map, emit ConnectionInfo findings.
///
/// # Arguments
/// * `files` - Files to scan for wrapper calls (typically language-filtered user files)
/// * `wrapper_map` - Built by build_wrapper_map() in Pass 1
/// * `service_roots` - For source_service attribution
///
/// # Returns
/// ExtractionResult with all ConnectionInfo findings from wrapper calls
pub fn detect_wrapper_calls(
    files: &[FileContext],
    wrapper_map: &WrapperMap,
    service_roots: &HashMap<PathBuf, String>,
) -> ExtractionResult {
    let mut connections = Vec::new();

    for file in files {
        for (line_idx, line) in file.content.lines().enumerate() {
            let trimmed = line.trim();

            // Skip comment lines
            if trimmed.starts_with("//")
                || trimmed.starts_with('#')
                || trimmed.starts_with("///")
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
            {
                continue;
            }

            // Check each wrapper in the map
            for (wrapper_name, info) in wrapper_map.iter() {
                // Skip seed entries (depth 0 — they are handled by pattern engine)
                if info.depth == 0 {
                    continue;
                }

                // Skip blocklisted names even if they somehow ended up in the map (WRAP-09)
                if WRAPPER_BLOCKLIST.contains(&wrapper_name.as_str()) {
                    continue;
                }

                // Check if this line contains a call to the wrapper
                let call_pattern = format!("{wrapper_name}(");
                if !line.contains(&call_pattern) {
                    continue;
                }

                // Extract the terminal function (seed) from the chain — last element
                let terminal = info
                    .chain
                    .last()
                    .cloned()
                    .unwrap_or_else(|| wrapper_name.clone());

                // Try to extract string argument (path/URL)
                let path = extract_string_arg_from_call(line, wrapper_name);

                let source_service = crate::plugin::scope_to_service(&file.path, service_roots)
                    .unwrap_or("")
                    .to_string();

                connections.push(ConnectionInfo {
                    source_service,
                    target_name: String::new(), // resolver fills this in from path matching
                    protocol: info.protocol.clone(),
                    method: None,
                    path,
                    source_file: format!("{}:{}", file.relative_path, line_idx + 1),
                    confidence: Confidence::Medium,
                    extraction_method: format!("wrapper_trace:{wrapper_name}→{terminal}"),
                    evidence: Some(trimmed.to_string()),
                });

                // Don't break — same line could call multiple wrappers (unusual but possible)
            }
        }
    }

    debug!(
        "Pass 2 detected {} wrapper calls across {} files",
        connections.len(),
        files.len()
    );

    ExtractionResult {
        connections,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests for normalize_template_literal()

    #[test]
    fn test_normalize_empty_string() {
        assert_eq!(normalize_template_literal(""), "");
    }

    #[test]
    fn test_normalize_no_interpolation() {
        assert_eq!(normalize_template_literal("/api/users"), "/api/users");
    }

    #[test]
    fn test_normalize_backtick_template() {
        assert_eq!(
            normalize_template_literal("`/api/v1/orgs/${orgId}/teams`"),
            "/api/v1/orgs/{param}/teams"
        );
    }

    #[test]
    fn test_normalize_python_fstring() {
        assert_eq!(
            normalize_template_literal("f\"/api/{org_id}/teams\""),
            "/api/{param}/teams"
        );
    }

    #[test]
    fn test_normalize_go_format_string() {
        assert_eq!(
            normalize_template_literal("\"/api/%s/teams\""),
            "/api/{param}/teams"
        );
    }

    #[test]
    fn test_normalize_ruby_string_interpolation() {
        assert_eq!(
            normalize_template_literal("\"/api/#{org_id}/teams\""),
            "/api/{param}/teams"
        );
    }

    #[test]
    fn test_normalize_rust_format_macro() {
        assert_eq!(
            normalize_template_literal("format!(\"/api/{}/teams\", org_id)"),
            "format!(\"/api/{param}/teams\", org_id)"
        );
    }

    #[test]
    fn test_normalize_multiple_params() {
        assert_eq!(
            normalize_template_literal("`/api/${a}/${b}`"),
            "/api/{param}/{param}"
        );
    }

    // Tests for WrapperMap

    #[test]
    fn test_wrapper_map_new() {
        let map = WrapperMap::new();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
    }

    #[test]
    fn test_wrapper_map_insert_and_contains() {
        let mut map = WrapperMap::new();
        assert!(!map.contains("fetch"));

        let info = WrapperInfo {
            protocol: "rest".to_string(),
            chain: vec!["fetch".to_string()],
            source: WrapperSource::UserCode {
                file: "lib/api.ts".to_string(),
                line: 3,
            },
            depth: 1,
        };
        map.insert("fetch".to_string(), info);

        assert!(map.contains("fetch"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn test_wrapper_map_get() {
        let mut map = WrapperMap::new();
        let info = WrapperInfo {
            protocol: "rest".to_string(),
            chain: vec!["fetch".to_string()],
            source: WrapperSource::UserCode {
                file: "lib/api.ts".to_string(),
                line: 3,
            },
            depth: 1,
        };
        map.insert("apiFetch".to_string(), info);

        let retrieved = map.get("apiFetch");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().protocol, "rest");
    }

    #[test]
    fn test_wrapper_map_len() {
        let mut map = WrapperMap::new();
        assert_eq!(map.len(), 0);

        map.insert(
            "fetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "lib.ts".to_string(),
                    line: 1,
                },
                depth: 1,
            },
        );
        assert_eq!(map.len(), 1);

        map.insert(
            "redis.connect".to_string(),
            WrapperInfo {
                protocol: "redis".to_string(),
                chain: vec!["redis.connect".to_string()],
                source: WrapperSource::UserCode {
                    file: "db.ts".to_string(),
                    line: 2,
                },
                depth: 1,
            },
        );
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_wrapper_map_default() {
        let map = WrapperMap::default();
        assert!(map.is_empty());
    }

    // Tests for Pass 1 — seed_from_patterns and build_wrapper_map

    #[test]
    fn test_seed_from_patterns_basic() {
        // Create a minimal PatternRegistry for testing with empty patterns
        let registry = crate::patterns::PatternRegistry::from_patterns(vec![], "test".to_string());

        let map = seed_from_patterns(&registry, "typescript", &[]);
        // With an empty registry, seeding should give us an empty map
        assert!(map.is_empty());
    }

    #[test]
    fn test_count_lines_empty() {
        assert_eq!(count_lines(""), 0);
    }

    #[test]
    fn test_count_lines_single() {
        assert_eq!(count_lines("let x = 1;"), 1);
    }

    #[test]
    fn test_count_lines_multiple() {
        assert_eq!(count_lines("let x = 1;\nlet y = 2;\nlet z = 3;"), 3);
    }

    #[test]
    fn test_check_function_skips_long_functions() {
        // Create a function body longer than 200 lines
        let mut long_body = String::new();
        for i in 0..205 {
            long_body.push_str(&format!("  let var{} = {};\n", i, i));
        }

        let file = FileContext {
            path: std::path::PathBuf::from("/test/file.ts"),
            relative_path: "file.ts".to_string(),
            content: std::sync::Arc::from("test"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "fetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 0,
            },
        );

        let mut found = Vec::new();
        check_function_and_add_to_wrapper_map(
            "longFunc", &long_body, &file, &map, false, "", &mut found,
        );

        // Should not add the function because it's > 200 lines
        assert!(found.is_empty());
    }

    #[test]
    fn test_check_function_detects_wrapper_call() {
        let file = FileContext {
            path: std::path::PathBuf::from("/test/file.ts"),
            relative_path: "file.ts".to_string(),
            content: std::sync::Arc::from("test"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "fetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 0,
            },
        );

        let fn_body = "function apiFetch(path) {\n  return fetch(path, opts);\n}";

        let mut found = Vec::new();
        check_function_and_add_to_wrapper_map(
            "apiFetch", fn_body, &file, &map, false, "", &mut found,
        );

        // Should find apiFetch as a wrapper around fetch
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "apiFetch");
        assert_eq!(found[0].1.protocol, "rest");
        assert_eq!(found[0].1.depth, 1);
        assert_eq!(found[0].1.chain, vec!["apiFetch", "fetch"]);
    }

    #[test]
    fn test_check_function_respects_depth_cap() {
        let file = FileContext {
            path: std::path::PathBuf::from("/test/file.ts"),
            relative_path: "file.ts".to_string(),
            content: std::sync::Arc::from("test"),
        };

        let mut map = WrapperMap::new();
        // Insert a wrapper at depth 2
        map.insert(
            "deepWrapper".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["a".to_string(), "fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 2,
            },
        );

        let fn_body = "function evenDeeper() { return deepWrapper(); }";

        let mut found = Vec::new();
        check_function_and_add_to_wrapper_map(
            "evenDeeper",
            fn_body,
            &file,
            &map,
            false,
            "",
            &mut found,
        );

        // Should not add because it would exceed depth cap of 2
        assert!(found.is_empty());
    }

    #[test]
    fn test_check_function_library_source() {
        let file = FileContext {
            path: std::path::PathBuf::from("/venv/site-packages/sdk/client.py"),
            relative_path: "sdk/client.py".to_string(),
            content: std::sync::Arc::from("test"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "httpx.post".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["httpx.post".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 0,
            },
        );

        let fn_body = "def append(self, event):\n  return httpx.post('/events', json=event)";

        let mut found = Vec::new();
        check_function_and_add_to_wrapper_map(
            "append",
            fn_body,
            &file,
            &map,
            true,
            "edgeworks_sdk",
            &mut found,
        );

        // Should find append as a library wrapper
        assert_eq!(found.len(), 1);
        match &found[0].1.source {
            WrapperSource::Library { lib_name, file } => {
                assert_eq!(lib_name, "edgeworks_sdk");
                assert_eq!(file, "sdk/client.py");
            }
            _ => panic!("Expected Library source"),
        }
    }

    #[test]
    fn test_build_wrapper_map_fixed_point() {
        // Create a simple FileContext for testing
        let user_file = FileContext {
            path: std::path::PathBuf::from("/test/api.ts"),
            relative_path: "api.ts".to_string(),
            content: std::sync::Arc::from(""), // We'll test with actual registry seeding
        };

        let registry = crate::patterns::PatternRegistry::from_patterns(vec![], "test".to_string());

        let map = build_wrapper_map(&[user_file], &[], &registry, "typescript");

        // With an empty registry, map should be empty
        assert!(map.is_empty());
    }

    // Tests for Pass 2 — detect_wrapper_calls

    #[test]
    fn test_extract_string_arg_simple_string() {
        let line = "apiFetch('/api/v1/teams')";
        let result = extract_string_arg_from_call(line, "apiFetch");
        assert_eq!(result, Some("/api/v1/teams".to_string()));
    }

    #[test]
    fn test_extract_string_arg_template_literal() {
        let line = "apiFetch(`/api/v1/orgs/${orgId}/teams`)";
        let result = extract_string_arg_from_call(line, "apiFetch");
        assert_eq!(result, Some("/api/v1/orgs/{param}/teams".to_string()));
    }

    #[test]
    fn test_extract_string_arg_fstring() {
        let line = "makeRequest(f\"/api/{org}/endpoint\")";
        let result = extract_string_arg_from_call(line, "makeRequest");
        assert_eq!(result, Some("/api/{param}/endpoint".to_string()));
    }

    #[test]
    fn test_extract_string_arg_no_string_arg() {
        let line = "apiFetch(buildUrl(id))";
        let result = extract_string_arg_from_call(line, "apiFetch");
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_string_arg_not_found() {
        let line = "someOtherCall('/api/path')";
        let result = extract_string_arg_from_call(line, "apiFetch");
        assert_eq!(result, None);
    }

    #[test]
    fn test_detect_wrapper_calls_simple() {
        let file = FileContext {
            path: std::path::PathBuf::from("/repo/hooks/useTeams.ts"),
            relative_path: "hooks/useTeams.ts".to_string(),
            content: std::sync::Arc::from("const teams = apiFetch('/api/v1/teams');"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "apiFetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["apiFetch".to_string(), "fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "lib/api.ts".to_string(),
                    line: 5,
                },
                depth: 1,
            },
        );

        let service_roots = HashMap::new();

        let result = detect_wrapper_calls(&[file], &map, &service_roots);

        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.path, Some("/api/v1/teams".to_string()));
        assert_eq!(conn.protocol, "rest");
        assert_eq!(conn.confidence, Confidence::Medium);
        assert_eq!(conn.extraction_method, "wrapper_trace:apiFetch→fetch");
        assert_eq!(
            conn.evidence,
            Some("const teams = apiFetch('/api/v1/teams');".to_string())
        );
        assert_eq!(conn.source_file, "hooks/useTeams.ts:1");
    }

    #[test]
    fn test_detect_wrapper_calls_template_literal() {
        let file = FileContext {
            path: std::path::PathBuf::from("/repo/hooks/useOrgs.ts"),
            relative_path: "hooks/useOrgs.ts".to_string(),
            content: std::sync::Arc::from("const orgs = apiFetch(`/api/v1/orgs/${orgId}/teams`);"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "apiFetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["apiFetch".to_string(), "fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "lib/api.ts".to_string(),
                    line: 5,
                },
                depth: 1,
            },
        );

        let service_roots = HashMap::new();

        let result = detect_wrapper_calls(&[file], &map, &service_roots);

        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.path, Some("/api/v1/orgs/{param}/teams".to_string()));
    }

    #[test]
    fn test_detect_wrapper_calls_skips_comments() {
        let file = FileContext {
            path: std::path::PathBuf::from("/repo/hooks/test.ts"),
            relative_path: "hooks/test.ts".to_string(),
            content: std::sync::Arc::from("// apiFetch('/api/v1/teams');\nconst x = 1;"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "apiFetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["apiFetch".to_string(), "fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "lib/api.ts".to_string(),
                    line: 5,
                },
                depth: 1,
            },
        );

        let service_roots = HashMap::new();

        let result = detect_wrapper_calls(&[file], &map, &service_roots);

        // Comment line should be skipped
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_detect_wrapper_calls_skips_seed_entries() {
        let file = FileContext {
            path: std::path::PathBuf::from("/repo/app.ts"),
            relative_path: "app.ts".to_string(),
            content: std::sync::Arc::from("const x = fetch('/api/data');"),
        };

        let mut map = WrapperMap::new();
        // Add fetch as a seed entry (depth 0)
        map.insert(
            "fetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 0, // Seed entry should be skipped by Pass 2
            },
        );

        let service_roots = HashMap::new();

        let result = detect_wrapper_calls(&[file], &map, &service_roots);

        // Seed entries should be skipped (handled by pattern engine instead)
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_detect_wrapper_calls_no_string_argument() {
        let file = FileContext {
            path: std::path::PathBuf::from("/repo/app.ts"),
            relative_path: "app.ts".to_string(),
            content: std::sync::Arc::from("const result = apiFetch(buildUrl(id));"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "apiFetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["apiFetch".to_string(), "fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "lib/api.ts".to_string(),
                    line: 5,
                },
                depth: 1,
            },
        );

        let service_roots = HashMap::new();

        let result = detect_wrapper_calls(&[file], &map, &service_roots);

        // Should still emit ConnectionInfo but with path = None
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.path, None);
        assert_eq!(conn.extraction_method, "wrapper_trace:apiFetch→fetch");
    }

    #[test]
    fn test_detect_wrapper_calls_multiple_wrappers_same_file() {
        let file = FileContext {
            path: std::path::PathBuf::from("/repo/app.ts"),
            relative_path: "app.ts".to_string(),
            content: std::sync::Arc::from(
                "const teams = apiFetch('/api/teams');\nconst events = eventLog('/events');",
            ),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "apiFetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["apiFetch".to_string(), "fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "lib/api.ts".to_string(),
                    line: 5,
                },
                depth: 1,
            },
        );
        map.insert(
            "eventLog".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["eventLog".to_string(), "fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "lib/logger.ts".to_string(),
                    line: 10,
                },
                depth: 1,
            },
        );

        let service_roots = HashMap::new();

        let result = detect_wrapper_calls(&[file], &map, &service_roots);

        // Should detect both wrapper calls
        assert_eq!(result.connections.len(), 2);
        assert_eq!(
            result.connections[0].extraction_method,
            "wrapper_trace:apiFetch→fetch"
        );
        assert_eq!(
            result.connections[1].extraction_method,
            "wrapper_trace:eventLog→fetch"
        );
    }

    #[test]
    fn test_detect_wrapper_calls_with_service_roots() {
        let file = FileContext {
            path: std::path::PathBuf::from("/repo/backend/src/api.ts"),
            relative_path: "backend/src/api.ts".to_string(),
            content: std::sync::Arc::from("const teams = apiFetch('/api/v1/teams');"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "apiFetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["apiFetch".to_string(), "fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "lib/api.ts".to_string(),
                    line: 5,
                },
                depth: 1,
            },
        );

        let mut service_roots = HashMap::new();
        service_roots.insert(
            std::path::PathBuf::from("/repo/backend"),
            "backend-service".to_string(),
        );

        let result = detect_wrapper_calls(&[file], &map, &service_roots);

        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.source_service, "backend-service");
    }

    #[test]
    fn test_depth_cap_allows_depth_two() {
        let file = FileContext {
            path: std::path::PathBuf::from("/test/file.ts"),
            relative_path: "file.ts".to_string(),
            content: std::sync::Arc::from("test"),
        };

        let mut map = WrapperMap::new();
        // Insert a wrapper at depth 1
        map.insert(
            "apiFetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["apiFetch".to_string(), "fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 1,
            },
        );

        let fn_body = "function hop2(path) { return apiFetch(path); }";

        let mut found = Vec::new();
        check_function_and_add_to_wrapper_map("hop2", fn_body, &file, &map, false, "", &mut found);

        // hop2 → apiFetch → fetch: new_depth = 2, should be allowed
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "hop2");
        assert_eq!(found[0].1.depth, 2);
    }

    #[test]
    fn test_depth_cap_blocks_depth_three() {
        let file = FileContext {
            path: std::path::PathBuf::from("/test/file.ts"),
            relative_path: "file.ts".to_string(),
            content: std::sync::Arc::from("test"),
        };

        let mut map = WrapperMap::new();
        // Insert a wrapper at depth 2
        map.insert(
            "hop2".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec![
                    "hop2".to_string(),
                    "apiFetch".to_string(),
                    "fetch".to_string(),
                ],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 2,
            },
        );

        let fn_body = "function hop3(path) { return hop2(path); }";

        let mut found = Vec::new();
        check_function_and_add_to_wrapper_map("hop3", fn_body, &file, &map, false, "", &mut found);

        // hop3 → hop2: new_depth = 3, exceeds cap — must not be added
        assert!(found.is_empty());
    }

    #[test]
    fn test_blocklist_blocks_init() {
        let file = FileContext {
            path: std::path::PathBuf::from("/test/file.py"),
            relative_path: "file.py".to_string(),
            content: std::sync::Arc::from("test"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "connect".to_string(),
            WrapperInfo {
                protocol: "grpc".to_string(),
                chain: vec!["connect".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 0,
            },
        );

        let fn_body = "def __init__(self):\n    self.conn = connect(host)";

        let mut found = Vec::new();
        check_function_and_add_to_wrapper_map(
            "__init__", fn_body, &file, &map, false, "", &mut found,
        );

        // __init__ is on the blocklist — must not be added as a wrapper
        assert!(found.is_empty());
    }

    #[test]
    fn test_blocklist_blocks_run() {
        let file = FileContext {
            path: std::path::PathBuf::from("/test/file.ts"),
            relative_path: "file.ts".to_string(),
            content: std::sync::Arc::from("test"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "fetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 0,
            },
        );

        let fn_body = "function run() { return fetch('/api/data'); }";

        let mut found = Vec::new();
        check_function_and_add_to_wrapper_map("run", fn_body, &file, &map, false, "", &mut found);

        // "run" is on the blocklist — must not be added as a wrapper
        assert!(found.is_empty());
    }

    #[test]
    fn test_blocklist_blocks_main() {
        let file = FileContext {
            path: std::path::PathBuf::from("/test/main.ts"),
            relative_path: "main.ts".to_string(),
            content: std::sync::Arc::from("test"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "fetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 0,
            },
        );

        let fn_body = "function main() { return fetch('/api/data'); }";

        let mut found = Vec::new();
        check_function_and_add_to_wrapper_map("main", fn_body, &file, &map, false, "", &mut found);

        // "main" is on the blocklist — must not be added as a wrapper
        assert!(found.is_empty());
    }

    #[test]
    fn test_blocklist_allows_real_wrapper() {
        let file = FileContext {
            path: std::path::PathBuf::from("/test/api.ts"),
            relative_path: "api.ts".to_string(),
            content: std::sync::Arc::from("test"),
        };

        let mut map = WrapperMap::new();
        map.insert(
            "fetch".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["fetch".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 0,
            },
        );

        let fn_body = "function api_get(path) { return fetch(path); }";

        let mut found = Vec::new();
        check_function_and_add_to_wrapper_map(
            "api_get", fn_body, &file, &map, false, "", &mut found,
        );

        // "api_get" is NOT on the blocklist — should be added normally
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "api_get");
    }

    #[test]
    fn test_detect_wrapper_calls_skips_blocklisted_names() {
        let file = FileContext {
            path: std::path::PathBuf::from("/test/service.py"),
            relative_path: "service.py".to_string(),
            content: std::sync::Arc::from("    __init__(arg)"),
        };

        // Simulate __init__ slipping into the map at depth 1
        let mut map = WrapperMap::new();
        map.insert(
            "__init__".to_string(),
            WrapperInfo {
                protocol: "rest".to_string(),
                chain: vec!["__init__".to_string(), "connect".to_string()],
                source: WrapperSource::UserCode {
                    file: "seed".to_string(),
                    line: 0,
                },
                depth: 1,
            },
        );

        let service_roots: HashMap<std::path::PathBuf, String> = HashMap::new();

        let result = detect_wrapper_calls(&[file], &map, &service_roots);

        // __init__ is on the blocklist — Pass 2 must emit no connections
        assert!(result.connections.is_empty());
    }
}
