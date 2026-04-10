use std::collections::HashMap;
use std::path::Path;

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, ExtractionResult};

use super::url_util::{is_connection_key, parse_url_value};

/// Environment file parser — reads .env files and emits ConnectionInfo for URL-valued keys.
pub struct EnvPlugin;

/// Parse a .env file's content into key/value pairs.
/// Strips comments, blank lines, `export ` prefix, and surrounding quotes.
fn parse_env_content(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        // Skip blank lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Strip `export ` prefix
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        // Split on first `=`
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let raw_val = line[eq_pos + 1..].trim();
            // Strip surrounding quotes (single or double)
            let val = if (raw_val.starts_with('"') && raw_val.ends_with('"'))
                || (raw_val.starts_with('\'') && raw_val.ends_with('\''))
            {
                raw_val[1..raw_val.len() - 1].to_string()
            } else {
                raw_val.to_string()
            };
            if !key.is_empty() {
                map.insert(key, val);
            }
        }
    }
    map
}

/// Derive source_service from a .env file's parent directory relative to the repo root.
fn derive_source_service(file_path: &Path, root: &Path) -> String {
    if let Some(parent) = file_path.parent() {
        if let Ok(rel) = parent.strip_prefix(root) {
            let s = rel.to_string_lossy().to_string();
            return s;
        }
    }
    String::new()
}

impl LanguagePlugin for EnvPlugin {
    fn name(&self) -> &str {
        "env"
    }

    fn file_patterns(&self) -> &[&str] {
        &[
            "**/.env",
            "**/.env.local",
            "**/.env.development",
            "**/.env.production",
            "**/.env.*",
        ]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        for file in &ctx.files {
            let kvs = parse_env_content(&file.content);
            let source_service = derive_source_service(&file.path, &ctx.root);

            for (key, val) in &kvs {
                if !is_connection_key(key) {
                    continue;
                }

                // Special handling for KAFKA_BROKERS: comma-separated host:port list
                if key == "KAFKA_BROKERS" {
                    // Take the first broker, split on `:`, take hostname
                    if let Some(first_broker) = val.split(',').next() {
                        let hostname = first_broker
                            .trim()
                            .split(':')
                            .next()
                            .unwrap_or(first_broker.trim())
                            .to_string();
                        if !hostname.is_empty() {
                            result.connections.push(ConnectionInfo {
                                source_service: source_service.clone(),
                                target_name: hostname,
                                protocol: "kafka".to_string(),
                                method: None,
                                path: None,
                                source_file: format!("{}:0", file.relative_path),
                                confidence: Confidence::High,
                                extraction_method: "spec:env".to_string(),
                                dependency: None,
                                evidence: Some(format!("{}={}", key, val)),
                                ..Default::default()
                            });
                        }
                    }
                    continue;
                }

                // URL-valued keys
                if let Some((protocol, hostname)) = parse_url_value(val) {
                    result.connections.push(ConnectionInfo {
                        source_service: source_service.clone(),
                        target_name: hostname,
                        protocol,
                        method: None,
                        path: None,
                        source_file: format!("{}:0", file.relative_path),
                        confidence: Confidence::High,
                        extraction_method: "spec:env".to_string(),
                        dependency: None,
                        evidence: Some(format!("{}={}", key, val)),
                        ..Default::default()
                    });
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::plugin::FileContext;
    use crate::vars::VariableStore;

    fn make_ctx(
        root: PathBuf,
        file_path: PathBuf,
        relative: &str,
        content: &str,
    ) -> ExtractionContext {
        let file = FileContext {
            path: file_path,
            relative_path: relative.to_string(),
            content: Arc::from(content),
        };
        ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_file_patterns() {
        let plugin = EnvPlugin;
        let patterns = plugin.file_patterns();
        assert!(patterns.contains(&"**/.env"));
        assert!(patterns.contains(&"**/.env.local"));
        assert!(patterns.contains(&"**/.env.development"));
        assert!(patterns.contains(&"**/.env.production"));
        assert!(patterns.contains(&"**/.env.*"));
    }

    #[test]
    fn test_extract_skips_non_connection_keys() {
        let plugin = EnvPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.env"),
            ".env",
            "APP_NAME=myapp\nDB_PORT=5432",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_always_run() {
        let plugin = EnvPlugin;
        assert!(plugin.always_run());
    }

    #[test]
    fn test_name() {
        let plugin = EnvPlugin;
        assert_eq!(plugin.name(), "env");
    }

    #[test]
    fn test_env_database_url() {
        let plugin = EnvPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.env"),
            ".env",
            "DATABASE_URL=postgres://db.host/mydb",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "postgresql");
        assert_eq!(conn.target_name, "db.host");
        assert_eq!(conn.source_service, "");
        assert_eq!(conn.extraction_method, "spec:env");
        assert!(conn.dependency.is_none());
    }

    #[test]
    fn test_env_redis_url() {
        let plugin = EnvPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.env"),
            ".env",
            "REDIS_URL=redis://cache.internal:6379",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "redis");
        assert_eq!(conn.target_name, "cache.internal");
    }

    #[test]
    fn test_env_kafka_brokers() {
        let plugin = EnvPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.env"),
            ".env",
            "KAFKA_BROKERS=broker1:9092,broker2:9092",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "kafka");
        assert_eq!(conn.target_name, "broker1");
    }

    #[test]
    fn test_env_service_scoped() {
        let plugin = EnvPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/services/api/.env"),
            "services/api/.env",
            "DATABASE_URL=postgres://db.host/mydb",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].source_service, "services/api");
    }

    #[test]
    fn test_env_skips_comments_and_blanks() {
        let plugin = EnvPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.env"),
            ".env",
            "# comment\n\nAPP_NAME=myapp",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_env_export_prefix() {
        let plugin = EnvPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.env"),
            ".env",
            "export DATABASE_URL=postgres://db.host/mydb",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].protocol, "postgresql");
        assert_eq!(result.connections[0].target_name, "db.host");
    }

    #[test]
    fn test_env_quoted_value() {
        let plugin = EnvPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/.env"),
            ".env",
            r#"DATABASE_URL="postgres://db.host/mydb""#,
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].protocol, "postgresql");
        assert_eq!(result.connections[0].target_name, "db.host");
    }
}
