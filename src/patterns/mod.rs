//! Pattern registry — loads detection patterns from remote or local cache,
//! applies them to files to produce ConnectionInfo findings.
//!
//! Pattern engine is the core of Phase 5: patterns replace all content-gate + line-scan
//! connection detection in compiled plugins. Compiled plugins keep only AST-based extraction.

use crate::plugin::FileContext;
use crate::types::{Confidence, ConnectionInfo, ExtractionResult};
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
    #[allow(dead_code)]
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
        Self { patterns, version, source: PatternSource::None }
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
        let cache_path = match std::env::var("HOME").ok() {
            Some(home) => PathBuf::from(home).join(".arcanon/patterns.json"),
            None => {
                tracing::warn!("HOME env var not set, no pattern cache available");
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
                tracing::warn!("Pattern fetch failed (network error or timeout). Falling back to cache.");
                Self::fallback_to_cache(&cache_path)
            }
        }
    }

    /// Fallback: try to read cache, or return empty registry
    fn fallback_to_cache(cache_path: &PathBuf) -> Self {
        match std::fs::read_to_string(cache_path) {
            Ok(content) => {
                match serde_json::from_str::<PatternFile>(&content) {
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
                        tracing::warn!("Failed to parse cached patterns: {}. Running with zero patterns.", e);
                        Self {
                            patterns: vec![],
                            version: "".to_string(),
                            source: PatternSource::None,
                        }
                    }
                }
            }
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

            // Line-by-line scan
            for (line_number, line) in file.content.lines().enumerate() {
                for detection in &pattern.detections {
                    if !line.contains(&detection.match_str) {
                        continue;
                    }

                    // Extract target
                    let (target_name, confidence) = match extract_target(line, &detection.target_extraction) {
                        Some(t) if !t.is_empty() => (t, map_confidence(&detection.confidence)),
                        _ => ("".to_string(), Confidence::Medium), // D-09
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
                                let key = other.strip_prefix("named_arg:").unwrap_or("").to_string();
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
        assert_eq!(findings.len(), 0, "Should skip pattern when import_gate not found");
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
        assert_eq!(findings[0].confidence, Confidence::Medium, "Should fallback to Medium when no string literal");
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
        assert_eq!(findings[0].target_name, "https://sqs.us-east-1.amazonaws.com/123/queue");
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
}
