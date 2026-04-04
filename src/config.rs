use serde::Deserialize;
use std::path::Path;

/// Top-level structure of .arcanon.toml.
/// All fields are optional — missing sections use defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ArcanonConfig {
    pub scanner: ScannerConfig,
    /// User-defined patterns from [[patterns]] array (D-10/D-11).
    /// These override remote patterns by ID.
    #[serde(default, rename = "patterns")]
    pub user_patterns: Vec<PatternOverride>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ScannerConfig {
    /// Default --hub-url (secrets stay in env vars, not here).
    pub hub_url: Option<String>,
    /// Default --project-slug.
    pub project_slug: Option<String>,
    pub exclude: ExcludeConfig,
    pub plugins: PluginsConfig,
    pub patterns: PatternsConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ExcludeConfig {
    /// Additional glob patterns to exclude (on top of built-in excludes).
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PluginsConfig {
    /// Plugin names to disable (e.g., ["ruby", "asyncapi"]).
    pub disabled: Option<Vec<String>>,
}

/// A user-defined detection pattern from [[patterns]] in .arcanon.toml (D-10).
/// Fields match the remote pattern schema exactly so patterns can be written
/// in the same format whether remote or local.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct PatternOverride {
    pub id: String,
    pub name: String,
    pub description: String,
    pub languages: Vec<String>,
    pub file_patterns: Vec<String>,
    pub import_gate: Vec<String>,
    pub detections: Vec<DetectionOverride>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct DetectionOverride {
    #[serde(rename = "match")]
    pub match_str: String,
    pub kind: String,
    pub protocol: String,
    pub confidence: String, // "high", "medium", "low" — plain String to avoid enum coupling
    pub target_extraction: String, // "none", "first_string_arg", "named_arg:KEY", "url_hostname"
}

/// [scanner.patterns] section (D-12: disabled blocklist).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PatternsConfig {
    /// Pattern IDs to disable — remote patterns with these IDs are excluded.
    pub disabled: Vec<String>,
}

/// Load `.arcanon.toml` from `scan_root` if it exists.
/// Returns `ArcanonConfig::default()` (all Nones) if the file is absent.
/// Logs a warning and returns default if the file is present but malformed.
pub fn load_file_config(scan_root: &Path) -> ArcanonConfig {
    let config_path = scan_root.join(".arcanon.toml");
    if !config_path.exists() {
        return ArcanonConfig::default();
    }
    match std::fs::read_to_string(&config_path) {
        Ok(contents) => match toml::from_str::<ArcanonConfig>(&contents) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("Warning: .arcanon.toml is invalid TOML: {e}. Using defaults.");
                ArcanonConfig::default()
            }
        },
        Err(e) => {
            eprintln!("Warning: failed to read .arcanon.toml: {e}. Using defaults.");
            ArcanonConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_missing_config_returns_default() {
        // Non-existent directory — no .arcanon.toml present
        let cfg = load_file_config(Path::new("/tmp/no-such-dir-arcanon-test"));
        assert!(cfg.scanner.hub_url.is_none());
        assert!(cfg.scanner.project_slug.is_none());
    }

    #[test]
    fn test_valid_config_parses() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("arcanon-config-test");
        std::fs::create_dir_all(&dir).unwrap();
        let config_path = dir.join(".arcanon.toml");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(
            f,
            r#"
[scanner]
project_slug = "test-project"
hub_url = "https://hub.example.com"
"#
        )
        .unwrap();

        let cfg = load_file_config(&dir);
        assert_eq!(cfg.scanner.project_slug.as_deref(), Some("test-project"));
        assert_eq!(
            cfg.scanner.hub_url.as_deref(),
            Some("https://hub.example.com")
        );

        std::fs::remove_file(&config_path).ok();
    }

    #[test]
    fn test_pattern_override_deserializes() {
        let toml_str = r#"
[[patterns]]
id = "internal-rpc"
name = "Internal RPC client"
description = "Company-specific RPC library"
languages = ["typescript"]
file_patterns = ["**/*.ts"]
import_gate = ["@company/rpc"]

[[patterns.detections]]
match = "createClient("
kind = "connection"
protocol = "grpc"
confidence = "high"
target_extraction = "first_string_arg"
"#;

        let cfg: ArcanonConfig = toml::from_str(toml_str).expect("failed to parse");
        assert_eq!(cfg.user_patterns.len(), 1);

        let pattern = &cfg.user_patterns[0];
        assert_eq!(pattern.id, "internal-rpc");
        assert_eq!(pattern.name, "Internal RPC client");
        assert_eq!(pattern.languages, vec!["typescript"]);
        assert_eq!(pattern.import_gate, vec!["@company/rpc"]);
        assert_eq!(pattern.detections.len(), 1);

        let detection = &pattern.detections[0];
        assert_eq!(detection.match_str, "createClient(");
        assert_eq!(detection.kind, "connection");
        assert_eq!(detection.protocol, "grpc");
        assert_eq!(detection.confidence, "high");
        assert_eq!(detection.target_extraction, "first_string_arg");
    }

    #[test]
    fn test_scanner_patterns_disabled_deserializes() {
        let toml_str = r#"
[scanner.patterns]
disabled = ["ts-axios", "py-boto3-sqs"]
"#;

        let cfg: ArcanonConfig = toml::from_str(toml_str).expect("failed to parse");
        assert_eq!(cfg.scanner.patterns.disabled.len(), 2);
        assert!(cfg
            .scanner
            .patterns
            .disabled
            .contains(&"ts-axios".to_string()));
        assert!(cfg
            .scanner
            .patterns
            .disabled
            .contains(&"py-boto3-sqs".to_string()));
    }

    #[test]
    fn test_missing_patterns_defaults_to_empty() {
        let toml_str = r#"
[scanner]
project_slug = "test-project"
"#;

        let cfg: ArcanonConfig = toml::from_str(toml_str).expect("failed to parse");
        assert_eq!(cfg.user_patterns.len(), 0);
        assert_eq!(cfg.scanner.patterns.disabled.len(), 0);
    }

    #[test]
    fn test_patterns_with_no_disabled_list() {
        let toml_str = r#"
[[patterns]]
id = "redis-py"
name = "redis-py"
description = "Python Redis client"
languages = ["python"]
file_patterns = ["**/*.py"]
import_gate = ["import redis", "from redis"]
"#;

        let cfg: ArcanonConfig = toml::from_str(toml_str).expect("failed to parse");
        assert_eq!(cfg.user_patterns.len(), 1);
        assert_eq!(cfg.scanner.patterns.disabled.len(), 0);
    }

    #[test]
    fn test_pattern_with_multiple_detections() {
        let toml_str = r#"
[[patterns]]
id = "boto3-sqs"
name = "AWS SQS (boto3)"
description = "AWS Simple Queue Service via boto3"
languages = ["python"]
file_patterns = ["**/*.py"]
import_gate = ["boto3"]

[[patterns.detections]]
match = "client('sqs')"
kind = "connection"
protocol = "sqs"
confidence = "high"
target_extraction = "none"

[[patterns.detections]]
match = "send_message("
kind = "connection"
protocol = "sqs"
confidence = "medium"
target_extraction = "named_arg:QueueUrl"
"#;

        let cfg: ArcanonConfig = toml::from_str(toml_str).expect("failed to parse");
        assert_eq!(cfg.user_patterns.len(), 1);

        let pattern = &cfg.user_patterns[0];
        assert_eq!(pattern.detections.len(), 2);

        let det1 = &pattern.detections[0];
        assert_eq!(det1.match_str, "client('sqs')");
        assert_eq!(det1.target_extraction, "none");

        let det2 = &pattern.detections[1];
        assert_eq!(det2.match_str, "send_message(");
        assert_eq!(det2.target_extraction, "named_arg:QueueUrl");
    }

    #[test]
    fn test_complete_config_with_patterns_and_disabled() {
        let toml_str = r#"
[scanner]
project_slug = "my-project"
hub_url = "https://hub.example.com"

[scanner.patterns]
disabled = ["ts-axios"]

[[patterns]]
id = "custom-client"
name = "Custom Client"
description = "Custom detection"
languages = ["typescript"]
file_patterns = ["**/*.ts"]
import_gate = ["custom-lib"]
"#;

        let cfg: ArcanonConfig = toml::from_str(toml_str).expect("failed to parse");
        assert_eq!(cfg.scanner.project_slug.as_deref(), Some("my-project"));
        assert_eq!(cfg.user_patterns.len(), 1);
        assert_eq!(cfg.scanner.patterns.disabled.len(), 1);
        assert_eq!(cfg.scanner.patterns.disabled[0], "ts-axios");
    }
}
