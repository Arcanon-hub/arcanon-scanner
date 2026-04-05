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

use std::collections::HashMap;

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
    pub source: WrapperSource,
    /// Depth in the wrapper chain (D-12: max 5)
    pub depth: usize,
}

/// Where a wrapper function was found.
#[derive(Debug, Clone)]
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
    let s = if s.starts_with("f\"") && s.ends_with('"') {
        &s[2..s.len() - 1]
    } else if s.starts_with("f'") && s.ends_with('\'') {
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
}
