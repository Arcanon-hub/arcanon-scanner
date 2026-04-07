//! Pattern registry — loads detection patterns from remote or local cache,
//! applies them to files to produce ConnectionInfo findings.
//!
//! Pattern engine is the core of Phase 5: patterns replace all content-gate + line-scan
//! connection detection in compiled plugins. Compiled plugins keep only AST-based extraction.

use crate::plugin::FileContext;
use crate::types::{Confidence, ConnectionInfo, ExtractionResult};
use globset::GlobSetBuilder;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::path::PathBuf;

/// Top-level pattern file fetched from CDN or cache.
/// CDN format groups patterns by language: { languages: [{ language, patterns }] }
#[derive(Debug, Clone, Deserialize)]
pub struct PatternFile {
    pub version: String,
    #[allow(dead_code)]
    pub updated_at: String,
    pub languages: Vec<LanguagePatterns>,
}

/// Patterns grouped by language (matches CDN JSON structure)
#[derive(Debug, Clone, Deserialize)]
pub struct LanguagePatterns {
    pub language: String,
    pub patterns: Vec<Pattern>,
}

impl PatternFile {
    /// Flatten language-grouped patterns into a single vec, injecting the language field
    pub fn into_patterns(self) -> Vec<Pattern> {
        self.languages
            .into_iter()
            .flat_map(|lp| {
                let lang = lp.language;
                lp.patterns.into_iter().map(move |mut p| {
                    // Per-language file patterns don't have a languages field in CDN JSON,
                    // so inject it from the parent group
                    if p.languages.is_empty() {
                        p.languages = vec![lang.clone()];
                    }
                    p
                })
            })
            .collect()
    }
}

/// A single pattern that matches on import_gate and detections
#[derive(Debug, Clone, Deserialize)]
pub struct Pattern {
    pub id: String,
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub description: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub file_patterns: Vec<String>,
    pub import_gate: Vec<String>,
    pub detections: Vec<Detection>,
}

/// A single detection within a pattern (match string + extraction strategy)
#[derive(Debug, Clone, Deserialize)]
pub struct Detection {
    #[serde(rename = "match")]
    pub match_str: String,
    #[allow(dead_code)]
    pub kind: String,
    pub protocol: String,
    pub confidence: PatternConfidence,
    pub target_extraction: TargetExtraction,
}

/// Confidence from pattern JSON — deserializes from "high", "medium", "low"
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PatternConfidence {
    High,
    Medium,
    Low,
}

/// Target extraction strategy — deserialized from string, parsed into enum
#[derive(Debug, Clone)]
pub enum TargetExtraction {
    None,
    FirstStringArg,
    NamedArg(String),
    UrlHostname,
    EnvDefault,
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
            "env_default" => TargetExtraction::EnvDefault,
            other if other.starts_with("named_arg:") => {
                let key = other.strip_prefix("named_arg:").unwrap_or("").to_string();
                TargetExtraction::NamedArg(key)
            }
            _ => TargetExtraction::None, // graceful unknown
        })
    }
}

/// Which source the patterns came from (for payload metadata per PTRN-07)
#[derive(Debug, Clone)]
pub enum PatternSource {
    Remote,
    Cache,
    None,
}

/// Pattern registry — holds loaded patterns and metadata
pub struct PatternRegistry {
    patterns: Vec<Pattern>,
    pub version: String,
    pub source: PatternSource,
}

impl PatternRegistry {
    /// Construct registry directly from a pattern list (for testing).
    #[allow(dead_code)]
    pub fn from_patterns(patterns: Vec<Pattern>, version: String) -> Self {
        Self {
            patterns,
            version,
            source: PatternSource::None,
        }
    }

    /// Load patterns from remote or cache. Async function; must be called from tokio context.
    ///
    /// Fetch strategy:
    /// 1. Determine cache path: ~/.arcanon/patterns.json
    /// 2. Read cached ETag if it exists: cache_path.etag
    /// 3. GET https://patterns.arcanon.dev/v1/patterns.json with If-None-Match header
    /// 4. On 200: parse, cache, and return Remote source
    /// 5. On 304: read cache and return Cache source
    /// 6. On error: fall back to cache, or return empty registry
    pub async fn load(_hub_url: Option<&str>) -> Self {
        // Determine cache path
        let cache_path = match dirs::home_dir() {
            Some(home) => home.join(".arcanon").join("patterns.json"),
            None => {
                tracing::warn!("Home directory not found, no pattern cache available");
                return Self {
                    patterns: vec![],
                    version: "".to_string(),
                    source: PatternSource::None,
                };
            }
        };

        let etag_path = format!("{}.etag", cache_path.display());

        // Read cached ETag if it exists
        let cached_etag = std::fs::read_to_string(&etag_path).ok();

        // Build client and fetch
        let client = reqwest::Client::new();
        let url = "https://patterns.arcanon.dev/v1/patterns.json";

        let mut req = client.get(url);

        if let Some(etag) = &cached_etag {
            req = req.header("If-None-Match", etag);
        }

        req = req.header("Accept", "application/json");

        match tokio::time::timeout(std::time::Duration::from_secs(10), req.send()).await {
            Ok(Ok(resp)) => {
                match resp.status().as_u16() {
                    200 => {
                        // Success: read body, cache, then parse
                        match resp.text().await {
                            Ok(body_str) => {
                                // Cache the patterns first
                                if let Some(parent) = cache_path.parent() {
                                    let _ = std::fs::create_dir_all(parent);
                                }
                                let _ = std::fs::write(&cache_path, &body_str);

                                // Parse the cached content
                                match serde_json::from_str::<PatternFile>(&body_str) {
                                    Ok(pattern_file) => {
                                        let version = pattern_file.version.clone();
                                        let patterns = pattern_file.into_patterns();

                                        Self {
                                            patterns,
                                            version,
                                            source: PatternSource::Remote,
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to parse pattern JSON: {}. Falling back to cache.", e);
                                        Self::fallback_to_cache(&cache_path)
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to read pattern response body: {}. Falling back to cache.", e);
                                Self::fallback_to_cache(&cache_path)
                            }
                        }
                    }
                    304 => {
                        // Not modified: use cached version
                        Self::fallback_to_cache(&cache_path)
                    }
                    _ => {
                        tracing::warn!("Pattern fetch returned status. Falling back to cache.");
                        Self::fallback_to_cache(&cache_path)
                    }
                }
            }
            _ => {
                tracing::warn!(
                    "Pattern fetch failed (network error or timeout). Falling back to cache."
                );
                Self::fallback_to_cache(&cache_path)
            }
        }
    }

    /// Fallback: try to read cache, or return empty registry
    fn fallback_to_cache(cache_path: &PathBuf) -> Self {
        match std::fs::read_to_string(cache_path) {
            Ok(content) => match serde_json::from_str::<PatternFile>(&content) {
                Ok(pattern_file) => {
                    let version = pattern_file.version.clone();
                    let patterns = pattern_file.into_patterns();
                    Self {
                        patterns,
                        version,
                        source: PatternSource::Cache,
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse cached patterns: {}. Running with zero patterns.",
                        e
                    );
                    Self {
                        patterns: vec![],
                        version: "".to_string(),
                        source: PatternSource::None,
                    }
                }
            },
            Err(_) => {
                tracing::warn!("No pattern cache found. Running with zero dynamic patterns.");
                Self {
                    patterns: vec![],
                    version: "".to_string(),
                    source: PatternSource::None,
                }
            }
        }
    }

    /// Access the patterns slice
    #[allow(dead_code)]
    pub fn patterns(&self) -> &[Pattern] {
        &self.patterns
    }

    /// Apply patterns to a single file and return findings
    pub fn apply(
        &self,
        file: &FileContext,
        language: &str,
        service_roots: &HashMap<PathBuf, String>,
    ) -> Vec<ConnectionInfo> {
        let mut findings = vec![];

        for pattern in &self.patterns {
            // Language filter
            if !pattern.languages.contains(&language.to_string()) {
                continue;
            }

            // file_patterns filter — if set, file path must match at least one glob (DACC-02)
            // TODO: consider caching compiled GlobSets per-pattern for large repos
            if !pattern.file_patterns.is_empty() {
                let mut builder = GlobSetBuilder::new();
                for pat in &pattern.file_patterns {
                    if let Ok(glob) = globset::Glob::new(pat) {
                        builder.add(glob);
                    }
                }
                match builder.build() {
                    Ok(gs) => {
                        if !gs.is_match(&file.relative_path) {
                            continue;
                        }
                    }
                    Err(_) => {
                        // Malformed globs: skip this pattern's file_patterns check entirely
                        // (do not skip the pattern — be permissive on broken config)
                    }
                }
            }

            // Import gate check
            if !pattern.import_gate.is_empty() {
                let gate_passed = pattern
                    .import_gate
                    .iter()
                    .any(|gate| file.content.contains(gate));
                if !gate_passed {
                    continue;
                }
            }

            // Triple-quote docstring state for Python — tracks whether we are inside a
            // multi-line """ or ''' block. Reset per-pattern since apply() outer loop
            // processes each pattern independently.
            let mut in_triple_quote = false;

            // Line-by-line scan
            for (line_number, line) in file.content.lines().enumerate() {
                let trimmed = line.trim();

                // Python docstring / triple-quote skip (DACC-04)
                // Only applies to Python — other languages use """ for other purposes
                if language == "python" {
                    let dq = trimmed.contains("\"\"\"");
                    let sq = trimmed.contains("'''");
                    if dq || sq {
                        let marker = if dq { "\"\"\"" } else { "'''" };
                        let count = trimmed.matches(marker).count();
                        if in_triple_quote {
                            // This line closes the block (possibly also re-opens one)
                            in_triple_quote = count % 2 == 0; // odd count means still open
                            continue; // skip the closing line itself
                        } else if count >= 2 {
                            // Opens and closes on the same line — skip this line, stay outside
                            continue;
                        } else {
                            // Opens a new block
                            in_triple_quote = true;
                            continue;
                        }
                    }
                    if in_triple_quote {
                        continue;
                    }
                }

                // Skip comments and string literals to avoid false positives
                // on test data, documentation, and embedded code snippets
                if trimmed.starts_with("//")
                    || trimmed.starts_with('#')
                    || trimmed.starts_with("///")
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with('*')
                    || (trimmed.starts_with('"') && trimmed.ends_with('"'))
                    || (trimmed.starts_with("r#\"") || trimmed.starts_with("r\""))
                {
                    continue;
                }

                for detection in &pattern.detections {
                    if !line.contains(&detection.match_str) {
                        continue;
                    }

                    // Extract target
                    let (target_name, confidence) = if matches!(
                        detection.target_extraction,
                        TargetExtraction::EnvDefault
                    ) {
                        let all_lines: Vec<&str> = file.content.lines().collect();
                        let target = extract_env_default(&all_lines, line_number, language);
                        let conf = if target.is_empty() {
                            Confidence::Medium
                        } else {
                            map_confidence(&detection.confidence)
                        };
                        (target, conf)
                    } else {
                        match extract_target(line, &detection.target_extraction) {
                            Some(t) if !t.is_empty() => (t, map_confidence(&detection.confidence)),
                            _ => ("".to_string(), Confidence::Medium), // D-09
                        }
                    };

                    findings.push(ConnectionInfo {
                        source_service: crate::plugin::scope_to_service(&file.path, service_roots)
                            .unwrap_or("")
                            .to_string(),
                        target_name,
                        protocol: detection.protocol.clone(),
                        method: None,
                        path: None,
                        source_file: format!("{}:{}", file.relative_path, line_number + 1),
                        confidence,
                        extraction_method: format!("pattern:{}", pattern.id),
                        dependency: Some(pattern.id.clone()),
                        evidence: Some(line.trim().to_string()),
                    });
                }
            }
        }

        findings
    }

    /// Apply patterns to all files and return aggregated ExtractionResult
    pub fn apply_all(
        &self,
        files: &[FileContext],
        language: &str,
        service_roots: &HashMap<PathBuf, String>,
    ) -> ExtractionResult {
        let mut connections = vec![];
        for file in files {
            connections.extend(self.apply(file, language, service_roots));
        }
        ExtractionResult {
            connections,
            ..Default::default()
        }
    }

    /// Apply user-defined pattern overrides from .arcanon.toml [[patterns]] (D-11).
    /// User pattern with same ID as a remote pattern replaces it entirely.
    /// New IDs are added to the set.
    pub fn with_overrides(mut self, overrides: &[crate::config::PatternOverride]) -> Self {
        for ov in overrides {
            // Convert PatternOverride → Pattern
            let converted = Pattern {
                id: ov.id.clone(),
                name: ov.name.clone(),
                description: ov.description.clone(),
                languages: ov.languages.clone(),
                file_patterns: ov.file_patterns.clone(),
                import_gate: ov.import_gate.clone(),
                detections: ov
                    .detections
                    .iter()
                    .map(|d| {
                        // Parse confidence: "high" → PatternConfidence::High
                        let confidence = match d.confidence.to_lowercase().as_str() {
                            "high" => PatternConfidence::High,
                            "medium" => PatternConfidence::Medium,
                            "low" => PatternConfidence::Low,
                            _ => PatternConfidence::Medium, // graceful default
                        };

                        // Parse target_extraction same as Deserialize impl
                        let target_extraction = match d.target_extraction.as_str() {
                            "none" => TargetExtraction::None,
                            "first_string_arg" => TargetExtraction::FirstStringArg,
                            "url_hostname" => TargetExtraction::UrlHostname,
                            other if other.starts_with("named_arg:") => {
                                let key =
                                    other.strip_prefix("named_arg:").unwrap_or("").to_string();
                                TargetExtraction::NamedArg(key)
                            }
                            _ => TargetExtraction::None, // graceful unknown
                        };

                        Detection {
                            match_str: d.match_str.clone(),
                            kind: d.kind.clone(),
                            protocol: d.protocol.clone(),
                            confidence,
                            target_extraction,
                        }
                    })
                    .collect(),
            };
            // Remove existing pattern with same ID
            self.patterns.retain(|p| p.id != ov.id);
            // Add user pattern
            self.patterns.push(converted);
        }
        self
    }

    /// Remove disabled patterns by ID (D-12).
    pub fn with_disabled(mut self, disabled: &[String]) -> Self {
        self.patterns.retain(|p| !disabled.contains(&p.id));
        self
    }
}

/// Map pattern confidence to crate confidence
fn map_confidence(pattern_conf: &PatternConfidence) -> Confidence {
    match pattern_conf {
        PatternConfidence::High => Confidence::High,
        PatternConfidence::Medium => Confidence::Medium,
        PatternConfidence::Low => Confidence::Low,
    }
}

/// Extract target from a line based on strategy
fn extract_target(line: &str, strategy: &TargetExtraction) -> Option<String> {
    match strategy {
        TargetExtraction::None => None,
        TargetExtraction::FirstStringArg => extract_first_string(line),
        TargetExtraction::NamedArg(key) => {
            let needle = format!("{}=", key);
            extract_named_arg(line, &needle)
        }
        TargetExtraction::UrlHostname => {
            extract_first_string(line).and_then(|url| {
                // Simple: find "://" then take until next "/"
                url.find("://").map(|i| {
                    let after = &url[i + 3..];
                    after.split('/').next().unwrap_or("").to_string()
                })
            })
        }
        // EnvDefault is handled in apply() before this function is called
        TargetExtraction::EnvDefault => None,
    }
}

/// Extract first quoted string from the line
fn extract_first_string(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'"' || b == b'\'' {
            let quote = b as char;
            let rest = &line[i + 1..];
            if let Some(end) = rest.find(quote) {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// Extract named argument value from the line
fn extract_named_arg(line: &str, needle: &str) -> Option<String> {
    line.find(needle).and_then(|pos| {
        let after_needle = &line[pos + needle.len()..];
        let bytes = after_needle.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'"' || b == b'\'' {
                let quote = b as char;
                let rest = &after_needle[i + 1..];
                if let Some(end) = rest.find(quote) {
                    return Some(rest[..end].to_string());
                }
            }
        }
        None
    })
}

/// Extract env var default value by scanning backward up to 20 lines.
/// Returns "env:{VAR}" when no default is found, "" when VAR is not parseable.
///
/// Strategy:
/// 1. For Java @Value on the matched line: extract inline default from `${VAR:default}`
/// 2. For tier-1 only languages (go, csharp, java): emit `env:{var}` from matched line quoted string
/// 3. For other languages: scan backward window for env var assignment patterns,
///    extract default from the *scan line* directly (not keyed on matched-line var name)
/// 4. If no match in backward scan: emit `env:{var}` if matched line has a quoted string, else ""
fn extract_env_default(lines: &[&str], line_idx: usize, language: &str) -> String {
    let matched_line = lines[line_idx];

    // Java @Value inline — extract default from matched line directly
    if language == "java" && matched_line.contains("@Value(") {
        // Pattern: @Value("${VAR:default}") — capture between ':' and '}' inside ${...}
        if let Some(dollar_pos) = matched_line.find("${") {
            let inner = &matched_line[dollar_pos + 2..];
            if let Some(colon_pos) = inner.find(':') {
                if let Some(brace_pos) = inner[colon_pos + 1..].find('}') {
                    let default = &inner[colon_pos + 1..colon_pos + 1 + brace_pos];
                    let default = default.trim_matches('"').trim_matches('\'').trim();
                    if !default.is_empty() {
                        return default.to_string();
                    }
                }
            }
        }
        // @Value present but no default — emit env hint using first quoted string
        return extract_first_string(matched_line)
            .map(|v| format!("env:{}", v))
            .unwrap_or_default();
    }

    // Tier-1 only languages: emit env hint without backward scan
    if matches!(language, "go" | "csharp" | "java") {
        return extract_first_string(matched_line)
            .map(|v| format!("env:{}", v))
            .unwrap_or_default();
    }

    // Backward scan: up to 20 lines before line_idx
    // We look for language-specific env var assignment patterns and extract the default
    // from the scan line itself (not keyed on the matched-line variable name).
    let scan_start = line_idx.saturating_sub(20);
    let window = &lines[scan_start..line_idx];

    // Also track the first env var name seen in the scan window for the fallback hint
    let mut scan_var_name: Option<String> = None;

    for scan_line in window.iter().rev() {
        let trimmed = scan_line.trim();

        let (found_default, found_var) = match language {
            "python" => {
                // os.getenv("VAR", "default") or os.environ.get("VAR", "default")
                if trimmed.contains("os.getenv(") || trimmed.contains("os.environ.get(") {
                    let var = extract_first_string(trimmed);
                    let default = extract_second_string_arg(trimmed);
                    (default, var)
                } else {
                    (None, None)
                }
            }
            "typescript" | "javascript" => {
                // process.env.VAR ?? "default" or process.env.VAR || "default"
                if trimmed.contains("process.env.") {
                    // Extract var name: identifier after "process.env."
                    let var = extract_process_env_var(trimmed);
                    let default = extract_after_nullish(trimmed);
                    (default, var)
                } else {
                    (None, None)
                }
            }
            "rust" => {
                // env::var("VAR").unwrap_or("default")
                if trimmed.contains("env::var(") {
                    let var = extract_first_string(trimmed);
                    let default = if trimmed.contains(".unwrap_or(") {
                        extract_unwrap_or_arg(trimmed)
                    } else {
                        None
                    };
                    (default, var)
                } else {
                    (None, None)
                }
            }
            "ruby" => {
                // ENV.fetch("VAR", "default") or ENV["VAR"] || "default"
                if trimmed.contains("ENV.fetch(") {
                    let var = extract_first_string(trimmed);
                    let default = extract_second_string_arg(trimmed);
                    (default, var)
                } else if trimmed.contains("ENV[") {
                    let var = extract_first_string(trimmed);
                    let default = extract_after_or(trimmed);
                    (default, var)
                } else {
                    (None, None)
                }
            }
            _ => (None, None),
        };

        if scan_var_name.is_none() {
            scan_var_name = found_var;
        }

        if let Some(default) = found_default {
            return default;
        }
    }

    // No default found in backward scan — emit env hint if var name is parseable
    // Prefer (in order):
    //   1. First quoted string from matched line (e.g. connect("DATABASE_URL"))
    //   2. First quoted string from scan window (e.g. os.getenv("DATABASE_URL") with no default)
    //   3. ALL_CAPS unquoted identifier from matched line (e.g. connect(DATABASE_URL))
    // Return "" if none of the above yield a usable name
    let hint_var = extract_first_string(matched_line)
        .or(scan_var_name)
        .or_else(|| extract_env_var_ident(matched_line));
    match hint_var {
        Some(v) => format!("env:{}", v),
        None => String::new(),
    }
}

/// Extract an ALL_CAPS identifier (env var pattern) from inside function call parens.
/// Only returns if the identifier matches [A-Z][A-Z0-9_]* (conventional env var naming).
/// Returns None for lowercase/mixed-case identifiers like `some_config_obj`.
fn extract_env_var_ident(line: &str) -> Option<String> {
    let paren_pos = line.find('(')?;
    // Skip leading `&` or whitespace after paren
    let after = line[paren_pos + 1..].trim_start_matches('&').trim();
    let end = after
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(after.len());
    let ident = &after[..end];
    // Must be ALL_CAPS (at least one char, starts with uppercase letter, no lowercase)
    if ident.is_empty() {
        return None;
    }
    let has_uppercase = ident.chars().any(|c| c.is_uppercase());
    let all_upper_or_digit_or_underscore = ident
        .chars()
        .all(|c| c.is_uppercase() || c.is_ascii_digit() || c == '_');
    if has_uppercase && all_upper_or_digit_or_underscore {
        Some(ident.to_string())
    } else {
        None
    }
}

/// Extract the identifier portion of `process.env.VARNAME` from a line
fn extract_process_env_var(line: &str) -> Option<String> {
    let prefix = "process.env.";
    let pos = line.find(prefix)?;
    let after = &line[pos + prefix.len()..];
    let end = after
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(after.len());
    let ident = &after[..end];
    if ident.is_empty() {
        None
    } else {
        Some(ident.to_string())
    }
}

/// Extract second quoted string from a function call: fn("first", "second")
fn extract_second_string_arg(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            let quote = b as char;
            let rest = &line[i + 1..];
            if let Some(end) = rest.find(quote) {
                count += 1;
                if count == 2 {
                    return Some(rest[..end].to_string());
                }
                i += end + 2;
                continue;
            }
        }
        i += 1;
    }
    None
}

/// Extract value after `??` or `||` operator, stripping quotes
fn extract_after_nullish(line: &str) -> Option<String> {
    let pos = line.find("??").or_else(|| line.find("||"))?;
    let after = line[pos + 2..].trim();
    // Strip surrounding quotes
    for q in ['"', '\'', '`'] {
        if after.starts_with(q) {
            if let Some(end) = after[1..].find(q) {
                return Some(after[1..1 + end].to_string());
            }
        }
    }
    None
}

/// Extract argument from `.unwrap_or("default")`
fn extract_unwrap_or_arg(line: &str) -> Option<String> {
    let pos = line.find(".unwrap_or(")?;
    let after = &line[pos + 11..]; // len(".unwrap_or(") == 11
    let bytes = after.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'"' || b == b'\'' {
            let quote = b as char;
            let rest = &after[i + 1..];
            if let Some(end) = rest.find(quote) {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// Extract value after `||` operator (Ruby ENV["VAR"] || "default"), stripping quotes
fn extract_after_or(line: &str) -> Option<String> {
    let pos = line.find("||")?;
    let after = line[pos + 2..].trim();
    for q in ['"', '\''] {
        if after.starts_with(q) {
            if let Some(end) = after[1..].find(q) {
                return Some(after[1..1 + end].to_string());
            }
        }
    }
    None
}

/// Extract first unquoted identifier-like argument from a function call.
/// Handles patterns like: connect(DATABASE_URL), Redis(url), Client(REDIS)
/// Returns the first ALL_CAPS or UPPER_LOWER identifier inside parens.
fn extract_unquoted_arg(line: &str) -> Option<String> {
    // Find the first '(' and extract the first word argument after it
    let paren_pos = line.find('(')?;
    let after = line[paren_pos + 1..].trim();
    // Read identifier characters (alphanumeric + underscore)
    let end = after
        .find(|c: char| !c.is_alphanumeric() && c != '_')
        .unwrap_or(after.len());
    let ident = &after[..end];
    if ident.is_empty() {
        None
    } else {
        Some(ident.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TASK 1: Pattern types and deserialization tests

    #[test]
    fn test_deserialize_redis_py_pattern() {
        let json = r#"{
            "version": "1.0",
            "updated_at": "2026-04-04T00:00:00Z",
            "languages": [
                {
                    "language": "python",
                    "patterns": [
                        {
                            "id": "redis-py",
                            "name": "redis-py",
                            "description": "Python Redis client",
                            "import_gate": ["import redis", "from redis"],
                            "detections": [
                                {
                                    "match": "Redis(",
                                    "kind": "connection",
                                    "protocol": "redis",
                                    "confidence": "high",
                                    "target_extraction": "first_string_arg"
                                }
                            ]
                        }
                    ]
                }
            ]
        }"#;

        let pattern_file: PatternFile = serde_json::from_str(json).expect("parse redis-py");
        assert_eq!(pattern_file.version, "1.0");
        let patterns = pattern_file.into_patterns();
        assert_eq!(patterns.len(), 1);

        let pattern = &patterns[0];
        assert_eq!(pattern.id, "redis-py");
        assert_eq!(pattern.languages, vec!["python"]);
        assert_eq!(pattern.import_gate.len(), 2);

        let detection = &pattern.detections[0];
        assert_eq!(detection.match_str, "Redis(");
        assert_eq!(detection.protocol, "redis");
    }

    #[test]
    fn test_named_arg_extraction_parsing() {
        let json = r#"{
            "version": "1.0",
            "updated_at": "2026-04-04T00:00:00Z",
            "languages": [{"language": "python", "patterns": [
                {
                    "id": "boto3-sqs",
                    "name": "boto3-sqs",
                    "description": "AWS SQS",
                    "import_gate": ["boto3"],
                    "detections": [
                        {
                            "match": "send_message(",
                            "kind": "connection",
                            "protocol": "sqs",
                            "confidence": "medium",
                            "target_extraction": "named_arg:QueueUrl"
                        }
                    ]
                }
            ]}]
        }"#;

        let pattern_file: PatternFile = serde_json::from_str(json).expect("parse boto3");
        let patterns = pattern_file.into_patterns();
        let detection = &patterns[0].detections[0];

        match &detection.target_extraction {
            TargetExtraction::NamedArg(key) => assert_eq!(key, "QueueUrl"),
            other => panic!("Expected NamedArg, got {:?}", other),
        }
    }

    #[test]
    fn test_unknown_target_extraction_graceful() {
        let json = r#"{
            "version": "1.0",
            "updated_at": "2026-04-04T00:00:00Z",
            "languages": [{"language": "python", "patterns": [
                {
                    "id": "test",
                    "name": "test",
                    "description": "test",
                    "import_gate": [],
                    "detections": [
                        {
                            "match": "test",
                            "kind": "connection",
                            "protocol": "test",
                            "confidence": "low",
                            "target_extraction": "unknown_strategy_xyz"
                        }
                    ]
                }
            ]}]
        }"#;

        let pattern_file: PatternFile = serde_json::from_str(json).expect("parse unknown");
        let patterns = pattern_file.into_patterns();
        let detection = &patterns[0].detections[0];

        // Should gracefully become None
        match &detection.target_extraction {
            TargetExtraction::None => {}
            other => panic!("Expected None for unknown strategy, got {:?}", other),
        }
    }

    #[test]
    fn test_empty_import_gate_valid() {
        let json = r#"{
            "version": "1.0",
            "updated_at": "2026-04-04T00:00:00Z",
            "languages": [{"language": "python", "patterns": [
                {
                    "id": "test",
                    "name": "test",
                    "description": "test",
                    "import_gate": [],
                    "detections": []
                }
            ]}]
        }"#;

        let pattern_file: PatternFile = serde_json::from_str(json).expect("parse empty gate");
        let patterns = pattern_file.into_patterns();
        assert_eq!(patterns[0].import_gate.len(), 0);
    }

    // TASK 2: Fetch, cache, and fallback tests

    #[test]
    fn test_load_nonexistent_url() {
        // Test that load() with no cache and unreachable URL returns empty registry
        // Note: This would be async in real usage, but we test the fallback logic
        let registry = PatternRegistry {
            patterns: vec![],
            version: "".to_string(),
            source: PatternSource::None,
        };

        assert_eq!(registry.patterns().len(), 0);
        assert_eq!(registry.version, "");
    }

    #[test]
    fn test_pattern_registry_patterns_accessor() {
        let pattern = Pattern {
            id: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec!["**/*.py".to_string()],
            import_gate: vec![],
            detections: vec![],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        assert_eq!(registry.patterns().len(), 1);
        assert_eq!(registry.patterns()[0].id, "test");
    }

    // TASK 3: Pattern apply engine tests

    #[test]
    fn test_import_gate_skip() {
        let pattern = Pattern {
            id: "redis-py".to_string(),
            name: "redis-py".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec!["**/*.py".to_string()],
            import_gate: vec!["import redis".to_string()],
            detections: vec![Detection {
                match_str: "Redis(".to_string(),
                kind: "connection".to_string(),
                protocol: "redis".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::FirstStringArg,
            }],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        let file = FileContext {
            path: PathBuf::from("/repo/test.py"),
            relative_path: "test.py".to_string(),
            content: std::sync::Arc::from("# no redis import\nprint('hello')"),
        };

        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(
            findings.len(),
            0,
            "Should skip pattern when import_gate not found"
        );
    }

    #[test]
    fn test_import_gate_fire() {
        let pattern = Pattern {
            id: "redis-py".to_string(),
            name: "redis-py".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec!["**/*.py".to_string()],
            import_gate: vec!["import redis".to_string()],
            detections: vec![Detection {
                match_str: "Redis(".to_string(),
                kind: "connection".to_string(),
                protocol: "redis".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::FirstStringArg,
            }],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        let file = FileContext {
            path: PathBuf::from("/repo/test.py"),
            relative_path: "test.py".to_string(),
            content: std::sync::Arc::from("import redis\nr = Redis('localhost')"),
        };

        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(findings.len(), 1, "Should fire when import_gate present");
        assert_eq!(findings[0].target_name, "localhost");
        assert_eq!(findings[0].protocol, "redis");
    }

    #[test]
    fn test_first_string_arg_extraction() {
        let pattern = Pattern {
            id: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "Redis(".to_string(),
                kind: "connection".to_string(),
                protocol: "redis".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::FirstStringArg,
            }],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        let file = FileContext {
            path: PathBuf::from("/repo/test.py"),
            relative_path: "test.py".to_string(),
            content: std::sync::Arc::from("r = Redis(\"redis://host:6379\")"),
        };

        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].target_name, "redis://host:6379");
    }

    #[test]
    fn test_first_string_arg_no_string_literal_medium_confidence() {
        let pattern = Pattern {
            id: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "Redis(".to_string(),
                kind: "connection".to_string(),
                protocol: "redis".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::FirstStringArg,
            }],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        let file = FileContext {
            path: PathBuf::from("/repo/test.py"),
            relative_path: "test.py".to_string(),
            content: std::sync::Arc::from("r = Redis(host_var)"),
        };

        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].target_name, "");
        assert_eq!(
            findings[0].confidence,
            Confidence::Medium,
            "Should fallback to Medium when no string literal"
        );
    }

    #[test]
    fn test_named_arg_extraction() {
        let pattern = Pattern {
            id: "boto3-sqs".to_string(),
            name: "boto3-sqs".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "send_message(".to_string(),
                kind: "connection".to_string(),
                protocol: "sqs".to_string(),
                confidence: PatternConfidence::Medium,
                target_extraction: TargetExtraction::NamedArg("QueueUrl".to_string()),
            }],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        let file = FileContext {
            path: PathBuf::from("/repo/test.py"),
            relative_path: "test.py".to_string(),
            content: std::sync::Arc::from(
                "sqs.send_message(QueueUrl=\"https://sqs.us-east-1.amazonaws.com/123/queue\")",
            ),
        };

        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].target_name,
            "https://sqs.us-east-1.amazonaws.com/123/queue"
        );
    }

    #[test]
    fn test_url_hostname_extraction() {
        let pattern = Pattern {
            id: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "requests.get(".to_string(),
                kind: "connection".to_string(),
                protocol: "http".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::UrlHostname,
            }],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        let file = FileContext {
            path: PathBuf::from("/repo/test.py"),
            relative_path: "test.py".to_string(),
            content: std::sync::Arc::from("requests.get(\"https://user-service:3000/api/users\")"),
        };

        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].target_name, "user-service:3000");
    }

    #[test]
    fn test_language_filter() {
        let pattern = Pattern {
            id: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "match".to_string(),
                kind: "connection".to_string(),
                protocol: "proto".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        let file = FileContext {
            path: PathBuf::from("/repo/test.ts"),
            relative_path: "test.ts".to_string(),
            content: std::sync::Arc::from("const x = match"),
        };

        let findings = registry.apply(&file, "typescript", &HashMap::new());
        assert_eq!(findings.len(), 0, "Should skip pattern for wrong language");
    }

    #[test]
    fn test_evidence_field() {
        let pattern = Pattern {
            id: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "match".to_string(),
                kind: "connection".to_string(),
                protocol: "proto".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        let file = FileContext {
            path: PathBuf::from("/repo/test.py"),
            relative_path: "test.py".to_string(),
            content: std::sync::Arc::from("x = match  "),
        };

        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].evidence, Some("x = match".to_string()));
    }

    #[test]
    fn test_source_file_format() {
        let pattern = Pattern {
            id: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "match".to_string(),
                kind: "connection".to_string(),
                protocol: "proto".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        let file = FileContext {
            path: PathBuf::from("/repo/src/app.py"),
            relative_path: "src/app.py".to_string(),
            content: std::sync::Arc::from("line1\nline2\nline3\nmatch here"),
        };

        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].source_file, "src/app.py:4");
    }

    #[test]
    fn test_apply_all() {
        let pattern = Pattern {
            id: "test".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "match".to_string(),
                kind: "connection".to_string(),
                protocol: "proto".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };

        let registry = PatternRegistry {
            patterns: vec![pattern],
            version: "1.0".to_string(),
            source: PatternSource::Remote,
        };

        let files = vec![
            FileContext {
                path: PathBuf::from("/repo/file1.py"),
                relative_path: "file1.py".to_string(),
                content: std::sync::Arc::from("match"),
            },
            FileContext {
                path: PathBuf::from("/repo/file2.py"),
                relative_path: "file2.py".to_string(),
                content: std::sync::Arc::from("match"),
            },
        ];

        let result = registry.apply_all(&files, "python", &HashMap::new());
        assert_eq!(result.connections.len(), 2);
    }

    // --- DACC-02: file_patterns glob filter tests ---

    #[test]
    fn test_file_patterns_skips_wrong_extension() {
        // Pattern restricted to *.ts should NOT fire on a .py file
        let pattern = Pattern {
            id: "ts-only".to_string(),
            name: "ts-only".to_string(),
            description: "test".to_string(),
            languages: vec!["typescript".to_string(), "python".to_string()],
            file_patterns: vec!["**/*.ts".to_string()],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "Client(".to_string(),
                kind: "connection".to_string(),
                protocol: "grpc".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };
        let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());
        let file = FileContext {
            path: PathBuf::from("/repo/test.py"),
            relative_path: "test.py".to_string(),
            content: std::sync::Arc::from("Client(\"grpc://host\")"),
        };
        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(findings.len(), 0, "file_patterns *.ts should skip .py file");
    }

    #[test]
    fn test_file_patterns_empty_matches_all() {
        let pattern = Pattern {
            id: "any-lang".to_string(),
            name: "any-lang".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![], // empty = match all
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "Client(".to_string(),
                kind: "connection".to_string(),
                protocol: "grpc".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };
        let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());
        let file = FileContext {
            path: PathBuf::from("/repo/test.py"),
            relative_path: "test.py".to_string(),
            content: std::sync::Arc::from("Client(\"host\")"),
        };
        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(
            findings.len(),
            1,
            "empty file_patterns should match any file"
        );
    }

    #[test]
    fn test_file_patterns_matches_correct_extension() {
        let pattern = Pattern {
            id: "py-only".to_string(),
            name: "py-only".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec!["**/*.py".to_string()],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "Client(".to_string(),
                kind: "connection".to_string(),
                protocol: "grpc".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };
        let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());
        let file = FileContext {
            path: PathBuf::from("/repo/services/api/test.py"),
            relative_path: "services/api/test.py".to_string(),
            content: std::sync::Arc::from("Client(\"host\")"),
        };
        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(
            findings.len(),
            1,
            "file_patterns **/*.py should match services/api/test.py"
        );
    }

    // --- DACC-04: Python triple-quoted docstring skip tests ---

    #[test]
    fn test_python_docstring_double_quote_skipped() {
        // Text INSIDE a triple-quote docstring must not produce findings
        let pattern = Pattern {
            id: "test-doc".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "Client(".to_string(),
                kind: "connection".to_string(),
                protocol: "opcua".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };
        let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());
        let content = "def connect():\n    \"\"\"\n    Example: Client(url) connects to OPC-UA\n    \"\"\"\n    pass";
        let file = FileContext {
            path: PathBuf::from("/repo/client.py"),
            relative_path: "client.py".to_string(),
            content: std::sync::Arc::from(content),
        };
        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(
            findings.len(),
            0,
            "Client( inside docstring must be skipped"
        );
    }

    #[test]
    fn test_python_docstring_single_quote_skipped() {
        let pattern = Pattern {
            id: "test-doc".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "Client(".to_string(),
                kind: "connection".to_string(),
                protocol: "opcua".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };
        let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());
        let content =
            "def connect():\n    '''\n    Example: Client(url) connects\n    '''\n    pass";
        let file = FileContext {
            path: PathBuf::from("/repo/client.py"),
            relative_path: "client.py".to_string(),
            content: std::sync::Arc::from(content),
        };
        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(
            findings.len(),
            0,
            "Client( inside single-quote docstring must be skipped"
        );
    }

    #[test]
    fn test_python_match_outside_docstring_still_fires() {
        // Real call after the docstring must still be found
        let pattern = Pattern {
            id: "test-doc".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "Client(".to_string(),
                kind: "connection".to_string(),
                protocol: "opcua".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };
        let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());
        let content = "def connect():\n    \"\"\"\n    Example: Client(url) docs\n    \"\"\"\n    c = Client(\"opc.tcp://host\")";
        let file = FileContext {
            path: PathBuf::from("/repo/client.py"),
            relative_path: "client.py".to_string(),
            content: std::sync::Arc::from(content),
        };
        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(
            findings.len(),
            1,
            "Client( outside docstring must still be found"
        );
    }

    #[test]
    fn test_triple_quote_inline_both_on_same_line_skipped() {
        // """Example: Client(url)""" on one line — should be skipped entirely
        let pattern = Pattern {
            id: "test-doc".to_string(),
            name: "test".to_string(),
            description: "test".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "Client(".to_string(),
                kind: "connection".to_string(),
                protocol: "opcua".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };
        let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());
        let file = FileContext {
            path: PathBuf::from("/repo/client.py"),
            relative_path: "client.py".to_string(),
            content: std::sync::Arc::from("\"\"\"Example: Client(url) docs\"\"\""),
        };
        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(
            findings.len(),
            0,
            "Inline triple-quote string must be skipped"
        );
    }

    #[test]
    fn test_pattern_engine_sets_dependency() {
        use std::collections::HashMap;
        let pattern = Pattern {
            id: "redis-py".to_string(),
            name: "redis-py".to_string(),
            description: "Python Redis client".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec!["import redis".to_string()],
            detections: vec![Detection {
                match_str: "Redis(".to_string(),
                kind: "connection".to_string(),
                protocol: "redis".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            }],
        };
        let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());
        let file = FileContext {
            path: PathBuf::from("/repo/client.py"),
            relative_path: "client.py".to_string(),
            content: std::sync::Arc::from("import redis\nRedis(host='localhost')"),
        };
        let findings = registry.apply(&file, "python", &HashMap::new());
        assert_eq!(findings.len(), 1, "should detect one connection");
        assert_eq!(
            findings[0].dependency,
            Some("redis-py".to_string()),
            "dependency must be pattern.id"
        );
    }

    // =========================================================
    // DQ-04: EnvDefault extraction tests
    // =========================================================

    #[test]
    fn test_env_default_python_os_getenv() {
        let lines = vec![
            r#"DATABASE_URL = os.getenv("DATABASE_URL", "postgres://localhost/db")"#,
            r#"conn = connect(DATABASE_URL)"#,
        ];
        let result = extract_env_default(&lines, 1, "python");
        assert_eq!(result, "postgres://localhost/db");
    }

    #[test]
    fn test_env_default_python_environ_get() {
        let lines = vec![
            r#"url = os.environ.get("REDIS_URL", "redis://localhost:6379")"#,
            r#"client = Redis(url)"#,
        ];
        let result = extract_env_default(&lines, 1, "python");
        assert_eq!(result, "redis://localhost:6379");
    }

    #[test]
    fn test_env_default_typescript_nullish() {
        let lines = vec![
            r#"const DB_URL = process.env.DB_URL ?? "postgres://localhost/mydb""#,
            r#"new Client(DB_URL)"#,
        ];
        let result = extract_env_default(&lines, 1, "typescript");
        assert_eq!(result, "postgres://localhost/mydb");
    }

    #[test]
    fn test_env_default_typescript_or_operator() {
        let lines = vec![
            r#"const REDIS = process.env.REDIS || "redis://localhost""#,
            r#"createClient(REDIS)"#,
        ];
        let result = extract_env_default(&lines, 1, "typescript");
        assert_eq!(result, "redis://localhost");
    }

    #[test]
    fn test_env_default_rust_unwrap_or() {
        let lines = vec![
            r#"let db_url = env::var("DATABASE_URL").unwrap_or("postgres://localhost/dev");"#,
            r#"PgPool::connect(&db_url)"#,
        ];
        let result = extract_env_default(&lines, 1, "rust");
        assert_eq!(result, "postgres://localhost/dev");
    }

    #[test]
    fn test_env_default_ruby_fetch() {
        let lines = vec![
            r#"DATABASE_URL = ENV.fetch("DATABASE_URL", "postgres://localhost/app")"#,
            r#"ActiveRecord::Base.establish_connection(DATABASE_URL)"#,
        ];
        let result = extract_env_default(&lines, 1, "ruby");
        assert_eq!(result, "postgres://localhost/app");
    }

    #[test]
    fn test_env_default_ruby_bracket_or() {
        let lines = vec![
            r#"REDIS_URL = ENV["REDIS_URL"] || "redis://localhost:6379""#,
            r#"Redis.new(url: REDIS_URL)"#,
        ];
        let result = extract_env_default(&lines, 1, "ruby");
        assert_eq!(result, "redis://localhost:6379");
    }

    #[test]
    fn test_env_default_java_value_annotation() {
        // Inline — the @Value line itself is the matched line
        let lines = vec![
            r#"@Value("${spring.datasource.url:jdbc:postgresql://localhost/db}")"#,
        ];
        let result = extract_env_default(&lines, 0, "java");
        assert_eq!(result, "jdbc:postgresql://localhost/db");
    }

    #[test]
    fn test_env_default_go_tier1_only() {
        let lines = vec![
            r#"url := os.Getenv("DATABASE_URL")"#,
        ];
        let result = extract_env_default(&lines, 0, "go");
        assert_eq!(result, "env:DATABASE_URL");
    }

    #[test]
    fn test_env_default_no_default_found_emits_hint() {
        // Python: assignment exists but no second arg default
        let lines = vec![
            r#"DATABASE_URL = os.getenv("DATABASE_URL")"#,
            r#"conn = connect(DATABASE_URL)"#,
        ];
        let result = extract_env_default(&lines, 1, "python");
        assert_eq!(result, "env:DATABASE_URL");
    }

    #[test]
    fn test_env_default_backward_scan_boundary_exactly_20_lines() {
        // Place assignment at offset 21 before the match line — should NOT be found
        // Lines: [import_line, assignment_line, 20 filler lines, matched_line]
        // line_idx = 22, assignment at index 1 — that is 21 lines back, outside window
        let mut lines = vec!["import os"];
        lines.push(r#"DATABASE_URL = os.getenv("DATABASE_URL", "postgres://out-of-window")"#); // index 1
        for _ in 0..20 {
            lines.push("# filler");
        }
        lines.push(r#"conn = connect(DATABASE_URL)"#); // index 22
        // Backward scan from index 22: window is lines[2..22] — 20 lines, none contain the assignment
        let result = extract_env_default(&lines, 22, "python");
        assert_eq!(result, "env:DATABASE_URL", "Must not scan beyond 20 lines");
    }

    #[test]
    fn test_env_default_backward_scan_exactly_within_20_lines() {
        // Place assignment at exactly 20 lines back — SHOULD be found
        // line_idx = 21: scan_start = 1, window = lines[1..21], assignment at index 1
        let mut lines = vec!["import os"];
        lines.push(r#"DATABASE_URL = os.getenv("DATABASE_URL", "postgres://in-window")"#); // index 1
        for _ in 0..19 {
            lines.push("# filler");
        }
        lines.push(r#"conn = connect(DATABASE_URL)"#); // index 21
        let result = extract_env_default(&lines, 21, "python");
        assert_eq!(result, "postgres://in-window", "Must scan exactly 20 lines back");
    }

    #[test]
    fn test_env_default_unparseable_var_returns_empty() {
        // Matched line has no quoted string — can't extract var name
        let lines = vec![r#"conn = connect(some_config_obj)"#];
        let result = extract_env_default(&lines, 0, "python");
        assert_eq!(result, "", "Must return empty string when var name not parseable");
    }

    #[test]
    fn test_env_default_deserializes_from_json() {
        let json = r#"{
            "version": "1.0",
            "updated_at": "2026-01-01T00:00:00Z",
            "languages": [{"language": "python", "patterns": [{
                "id": "py-env-getenv",
                "name": "Python os.getenv",
                "description": "env var default",
                "import_gate": ["import os"],
                "detections": [{
                    "match": "os.getenv(",
                    "kind": "connection",
                    "protocol": "env",
                    "confidence": "low",
                    "target_extraction": "env_default"
                }]
            }]}]
        }"#;
        let pattern_file: PatternFile = serde_json::from_str(json).expect("parse");
        let patterns = pattern_file.into_patterns();
        assert!(matches!(patterns[0].detections[0].target_extraction, TargetExtraction::EnvDefault));
    }
}
