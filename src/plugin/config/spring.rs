use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use tracing::warn;

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, ExtractionResult};

use super::url_util::scheme_to_protocol;

/// Spring Boot configuration parser — reads application.properties and application.yml
/// files to extract datasource, cache, messaging, and broker connections.
pub struct SpringPlugin;

// ---------------------------------------------------------------------------
// Private YAML deserialization structs
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct SpringConfig {
    spring: Option<SpringSection>,
}

#[derive(Deserialize, Default)]
struct SpringSection {
    datasource: Option<DatasourceConfig>,
    redis: Option<RedisConfig>,
    data: Option<DataConfig>,
    kafka: Option<KafkaConfig>,
    rabbitmq: Option<RabbitmqConfig>,
}

#[derive(Deserialize, Default)]
struct DatasourceConfig {
    url: Option<String>,
    host: Option<String>,
}

#[derive(Deserialize, Default)]
struct RedisConfig {
    host: Option<String>,
}

#[derive(Deserialize, Default)]
struct DataConfig {
    redis: Option<RedisConfig>,
}

#[derive(Deserialize, Default)]
struct KafkaConfig {
    #[serde(rename = "bootstrap-servers")]
    bootstrap_servers: Option<String>,
}

#[derive(Deserialize, Default)]
struct RabbitmqConfig {
    host: Option<String>,
}

// ---------------------------------------------------------------------------
// Private helper functions
// ---------------------------------------------------------------------------

/// Extract (protocol, hostname) from a JDBC URL.
/// JDBC URLs like "jdbc:postgresql://db.host/mydb" are not RFC 3986 URLs
/// and cannot be parsed with url::Url::parse().
fn extract_jdbc_hostname(jdbc_url: &str) -> Option<(String, String)> {
    if !jdbc_url.starts_with("jdbc:") {
        return None;
    }
    let rest = &jdbc_url[5..]; // e.g. "postgresql://db.host/mydb"
    let (subprotocol, conn_str) = rest.split_once("://")?;
    // Extract hostname from "db.host/mydb" or "db.host:5432/mydb"
    let hostname = conn_str.split(':').next()?.split('/').next()?.to_string();
    if hostname.is_empty() {
        return None;
    }
    Some((scheme_to_protocol(subprotocol), hostname))
}

/// Parse a Spring .properties file's content into key/value pairs.
/// Strips comments, blank lines, and surrounding quotes.
/// Does NOT strip "export " prefix (not a Spring pattern).
fn parse_properties_content(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        // Skip blank lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
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

/// Derive source_service from a Spring config file's parent directory relative to the repo root.
fn derive_source_service(file_path: &Path, root: &Path) -> String {
    if let Some(parent) = file_path.parent() {
        if let Ok(rel) = parent.strip_prefix(root) {
            return rel.to_string_lossy().to_string();
        }
    }
    String::new()
}

/// Build a ConnectionInfo for a Spring-detected connection.
fn emit_connection(
    source_service: String,
    target_name: String,
    protocol: String,
    source_file: String,
    key: &str,
    val: &str,
) -> ConnectionInfo {
    ConnectionInfo {
        source_service,
        target_name,
        protocol,
        method: None,
        path: None,
        source_file,
        confidence: Confidence::High,
        extraction_method: "spec:spring".to_string(),
        dependency: None,
        evidence: Some(format!("{}={}", key, val)),
    }
}

/// Extract the hostname from a Kafka bootstrap-servers string.
/// Takes the first broker from a comma-separated list and strips the port.
fn extract_kafka_hostname(bootstrap_servers: &str) -> Option<String> {
    let first = bootstrap_servers.split(',').next()?;
    let hostname = first.trim().split(':').next()?.to_string();
    if hostname.is_empty() {
        None
    } else {
        Some(hostname)
    }
}

/// Convert a flat key-value map of Spring properties into ConnectionInfo entries.
fn process_spring_keys(
    kvs: &HashMap<String, String>,
    source_service: &str,
    source_file: &str,
) -> Vec<ConnectionInfo> {
    let mut connections = Vec::new();

    // spring.datasource.url — JDBC URL
    if let Some(val) = kvs.get("spring.datasource.url") {
        if let Some((protocol, hostname)) = extract_jdbc_hostname(val) {
            connections.push(emit_connection(
                source_service.to_string(),
                hostname,
                protocol,
                source_file.to_string(),
                "spring.datasource.url",
                val,
            ));
        }
    }

    // spring.datasource.host — direct hostname
    if let Some(val) = kvs.get("spring.datasource.host") {
        if !val.is_empty() {
            connections.push(emit_connection(
                source_service.to_string(),
                val.clone(),
                "datasource".to_string(),
                source_file.to_string(),
                "spring.datasource.host",
                val,
            ));
        }
    }

    // spring.redis.host — direct hostname
    if let Some(val) = kvs.get("spring.redis.host") {
        if !val.is_empty() {
            connections.push(emit_connection(
                source_service.to_string(),
                val.clone(),
                "redis".to_string(),
                source_file.to_string(),
                "spring.redis.host",
                val,
            ));
        }
    }

    // spring.data.redis.host — direct hostname
    if let Some(val) = kvs.get("spring.data.redis.host") {
        if !val.is_empty() {
            connections.push(emit_connection(
                source_service.to_string(),
                val.clone(),
                "redis".to_string(),
                source_file.to_string(),
                "spring.data.redis.host",
                val,
            ));
        }
    }

    // spring.kafka.bootstrap-servers — CSV broker list, take first hostname
    if let Some(val) = kvs.get("spring.kafka.bootstrap-servers") {
        if let Some(hostname) = extract_kafka_hostname(val) {
            connections.push(emit_connection(
                source_service.to_string(),
                hostname,
                "kafka".to_string(),
                source_file.to_string(),
                "spring.kafka.bootstrap-servers",
                val,
            ));
        }
    }

    // spring.rabbitmq.host — direct hostname
    if let Some(val) = kvs.get("spring.rabbitmq.host") {
        if !val.is_empty() {
            connections.push(emit_connection(
                source_service.to_string(),
                val.clone(),
                "rabbitmq".to_string(),
                source_file.to_string(),
                "spring.rabbitmq.host",
                val,
            ));
        }
    }

    connections
}

// ---------------------------------------------------------------------------
// LanguagePlugin implementation
// ---------------------------------------------------------------------------

impl LanguagePlugin for SpringPlugin {
    fn name(&self) -> &str {
        "spring"
    }

    fn file_patterns(&self) -> &[&str] {
        &[
            "**/application.properties",
            "**/application-*.properties",
            "**/application.yml",
            "**/application-*.yml",
        ]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        for file in &ctx.files {
            let source_service = derive_source_service(&file.path, &ctx.root);
            let source_file = format!("{}:0", file.relative_path);

            let is_yaml =
                file.relative_path.ends_with(".yml") || file.relative_path.ends_with(".yaml");

            if is_yaml {
                match serde_yaml_bw::from_str::<SpringConfig>(&file.content) {
                    Ok(config) => {
                        if let Some(spring) = config.spring {
                            let mut kvs = HashMap::new();

                            // Flatten YAML struct into the same key format as properties
                            if let Some(ds) = spring.datasource {
                                if let Some(url) = ds.url {
                                    kvs.insert("spring.datasource.url".to_string(), url);
                                }
                                if let Some(host) = ds.host {
                                    kvs.insert("spring.datasource.host".to_string(), host);
                                }
                            }
                            if let Some(redis) = spring.redis {
                                if let Some(host) = redis.host {
                                    kvs.insert("spring.redis.host".to_string(), host);
                                }
                            }
                            if let Some(data) = spring.data {
                                if let Some(redis) = data.redis {
                                    if let Some(host) = redis.host {
                                        kvs.insert("spring.data.redis.host".to_string(), host);
                                    }
                                }
                            }
                            if let Some(kafka) = spring.kafka {
                                if let Some(bs) = kafka.bootstrap_servers {
                                    kvs.insert("spring.kafka.bootstrap-servers".to_string(), bs);
                                }
                            }
                            if let Some(rmq) = spring.rabbitmq {
                                if let Some(host) = rmq.host {
                                    kvs.insert("spring.rabbitmq.host".to_string(), host);
                                }
                            }

                            let conns = process_spring_keys(&kvs, &source_service, &source_file);
                            result.connections.extend(conns);
                        }
                    }
                    Err(e) => {
                        warn!("spring: failed to parse YAML {}: {}", file.relative_path, e);
                    }
                }
            } else {
                // Properties file
                let kvs = parse_properties_content(&file.content);
                let conns = process_spring_keys(&kvs, &source_service, &source_file);
                result.connections.extend(conns);
            }
        }

        result
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Plugin trait tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_name() {
        let plugin = SpringPlugin;
        assert_eq!(plugin.name(), "spring");
    }

    #[test]
    fn test_always_run() {
        let plugin = SpringPlugin;
        assert!(plugin.always_run());
    }

    #[test]
    fn test_file_patterns() {
        let plugin = SpringPlugin;
        let patterns = plugin.file_patterns();
        assert!(patterns.contains(&"**/application.properties"));
        assert!(patterns.contains(&"**/application-*.properties"));
        assert!(patterns.contains(&"**/application.yml"));
        assert!(patterns.contains(&"**/application-*.yml"));
    }

    // -----------------------------------------------------------------------
    // JDBC extraction unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_jdbc_postgresql() {
        let result = extract_jdbc_hostname("jdbc:postgresql://db.host/mydb");
        assert_eq!(
            result,
            Some(("postgresql".to_string(), "db.host".to_string()))
        );
    }

    #[test]
    fn test_jdbc_mysql() {
        let result = extract_jdbc_hostname("jdbc:mysql://db.host:3306/mydb");
        assert_eq!(result, Some(("mysql".to_string(), "db.host".to_string())));
    }

    #[test]
    fn test_jdbc_non_jdbc() {
        // Non-JDBC URL — must return None
        let result = extract_jdbc_hostname("postgres://db.host/mydb");
        assert_eq!(result, None);
    }

    #[test]
    fn test_jdbc_empty_hostname() {
        // JDBC URL with empty hostname
        let result = extract_jdbc_hostname("jdbc:postgresql:///mydb");
        assert_eq!(result, None);
    }

    // -----------------------------------------------------------------------
    // Properties file integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_properties_datasource_url() {
        let plugin = SpringPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.properties"),
            "application.properties",
            "spring.datasource.url=jdbc:postgresql://db.host/mydb",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "postgresql");
        assert_eq!(conn.target_name, "db.host");
        assert_eq!(conn.extraction_method, "spec:spring");
    }

    #[test]
    fn test_properties_datasource_host() {
        let plugin = SpringPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.properties"),
            "application.properties",
            "spring.datasource.host=db.host",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "datasource");
        assert_eq!(conn.target_name, "db.host");
    }

    #[test]
    fn test_properties_redis_host() {
        let plugin = SpringPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.properties"),
            "application.properties",
            "spring.redis.host=redis.host",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "redis");
        assert_eq!(conn.target_name, "redis.host");
    }

    #[test]
    fn test_properties_data_redis_host() {
        let plugin = SpringPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.properties"),
            "application.properties",
            "spring.data.redis.host=redis.host",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "redis");
        assert_eq!(conn.target_name, "redis.host");
    }

    #[test]
    fn test_properties_kafka_bootstrap() {
        let plugin = SpringPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.properties"),
            "application.properties",
            "spring.kafka.bootstrap-servers=broker1:9092,broker2:9092",
        );
        let result = plugin.extract(&ctx);
        // Must emit exactly 1 connection (first broker hostname only)
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "kafka");
        assert_eq!(conn.target_name, "broker1");
    }

    #[test]
    fn test_properties_rabbitmq_host() {
        let plugin = SpringPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.properties"),
            "application.properties",
            "spring.rabbitmq.host=rabbit.host",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "rabbitmq");
        assert_eq!(conn.target_name, "rabbit.host");
    }

    #[test]
    fn test_properties_non_spring_key_skipped() {
        let plugin = SpringPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.properties"),
            "application.properties",
            "server.port=8080",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_properties_comments_and_blanks() {
        let plugin = SpringPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.properties"),
            "application.properties",
            "# comment\n\n# another comment\n",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_properties_source_service_scoped() {
        let plugin = SpringPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/services/api/application.properties"),
            "services/api/application.properties",
            "spring.datasource.url=jdbc:postgresql://db.host/mydb",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].source_service, "services/api");
    }

    #[test]
    fn test_properties_kafka_single_host() {
        let plugin = SpringPlugin;
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.properties"),
            "application.properties",
            "spring.kafka.bootstrap-servers=broker1:9092",
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].target_name, "broker1");
    }

    // -----------------------------------------------------------------------
    // YAML integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_yaml_redis_host() {
        let plugin = SpringPlugin;
        let content = "spring:\n  redis:\n    host: redis.host\n";
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.yml"),
            "application.yml",
            content,
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "redis");
        assert_eq!(conn.target_name, "redis.host");
    }

    #[test]
    fn test_yaml_datasource_url() {
        let plugin = SpringPlugin;
        let content = "spring:\n  datasource:\n    url: jdbc:postgresql://db.host/mydb\n";
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.yml"),
            "application.yml",
            content,
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "postgresql");
        assert_eq!(conn.target_name, "db.host");
    }

    #[test]
    fn test_yaml_rabbitmq_host() {
        let plugin = SpringPlugin;
        let content = "spring:\n  rabbitmq:\n    host: rabbit.host\n";
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.yml"),
            "application.yml",
            content,
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "rabbitmq");
        assert_eq!(conn.target_name, "rabbit.host");
    }

    #[test]
    fn test_yaml_kafka_bootstrap() {
        let plugin = SpringPlugin;
        let content = "spring:\n  kafka:\n    bootstrap-servers: broker1:9092,broker2:9092\n";
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.yml"),
            "application.yml",
            content,
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "kafka");
        assert_eq!(conn.target_name, "broker1");
    }

    #[test]
    fn test_yaml_missing_sections() {
        let plugin = SpringPlugin;
        // YAML with no spring.datasource key
        let content = "server:\n  port: 8080\n";
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.yml"),
            "application.yml",
            content,
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_yaml_empty_spring_block() {
        let plugin = SpringPlugin;
        // spring: {} — no sub-keys
        let content = "spring: {}\n";
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.yml"),
            "application.yml",
            content,
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_yaml_malformed() {
        let plugin = SpringPlugin;
        // Invalid YAML — must not panic, must return 0 connections
        let content = "spring:\n  : bad: yaml: {\n";
        let ctx = make_ctx(
            PathBuf::from("/repo"),
            PathBuf::from("/repo/application.yml"),
            "application.yml",
            content,
        );
        let result = plugin.extract(&ctx);
        assert_eq!(result.connections.len(), 0);
    }
}
