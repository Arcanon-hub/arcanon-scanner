use serde::Deserialize;
use std::path::Path;

/// Top-level structure of .arcanon.toml.
/// All fields are optional — missing sections use defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ArcanonConfig {
    pub scanner: ScannerConfig,
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
    use std::path::PathBuf;

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
}
