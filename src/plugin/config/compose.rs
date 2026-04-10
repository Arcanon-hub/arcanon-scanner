use std::collections::HashMap;

use serde::Deserialize;
use tracing::warn;

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, ExtractionResult, ServiceInfo};

use super::url_util::{is_connection_key, parse_url_value};

/// Docker Compose manifest parser.
pub struct ComposePlugin;

#[derive(Deserialize, Default)]
struct ComposeFile {
    #[serde(default)]
    services: HashMap<String, ComposeService>,
}

/// environment: can be a list ["KEY=value"] or map {KEY: value}
#[derive(Deserialize, Default)]
#[serde(untagged)]
enum ComposeEnv {
    #[default]
    None,
    List(Vec<String>),
    Map(HashMap<String, serde_yaml_bw::Value>),
}

#[derive(Deserialize, Default)]
struct ComposeService {
    #[serde(default)]
    depends_on: DependsOn,
    #[serde(default)]
    environment: ComposeEnv,
}

/// depends_on can be either a list of strings or a map {service: {condition: ...}}
#[derive(Deserialize, Default)]
#[serde(untagged)]
enum DependsOn {
    #[default]
    None,
    List(Vec<String>),
    Map(HashMap<String, serde_yaml_bw::Value>),
}

impl DependsOn {
    fn service_names(&self) -> Vec<String> {
        match self {
            DependsOn::None => vec![],
            DependsOn::List(v) => v.clone(),
            DependsOn::Map(m) => m.keys().cloned().collect(),
        }
    }
}

impl LanguagePlugin for ComposePlugin {
    fn name(&self) -> &str {
        "compose"
    }

    fn file_patterns(&self) -> &[&str] {
        &[
            "**/docker-compose*.yml",
            "**/docker-compose*.yaml",
            "**/compose*.yml",
            "**/compose*.yaml",
        ]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        for file in &ctx.files {
            // Attempt to parse the compose file
            let compose: ComposeFile = match serde_yaml_bw::from_str(&file.content) {
                Ok(c) => c,
                Err(e) => {
                    warn!("compose: failed to parse {}: {}", file.relative_path, e);
                    continue;
                }
            };

            // Derive root_path: parent directory of the compose file
            let root_path = if let Some(parent) = file.path.parent() {
                if let Ok(rel) = parent.strip_prefix(&ctx.root) {
                    rel.to_string_lossy().to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            // Build ServiceInfo for each service
            for service_name in compose.services.keys() {
                result.services.push(ServiceInfo {
                    name: service_name.clone(),
                    root_path: root_path.clone(),
                    language: String::new(),
                    service_type: "service".to_string(),
                    boundary_entry: None,
                    confidence: Confidence::High,
                    extraction_method: "compose".to_string(),
                    ..Default::default()
                });
            }

            // Build ConnectionInfo for each depends_on entry
            for (service_name, service) in &compose.services {
                for dep_name in service.depends_on.service_names() {
                    result.connections.push(ConnectionInfo {
                        source_service: service_name.clone(),
                        target_name: dep_name.clone(),
                        protocol: "compose-depends_on".to_string(),
                        method: None,
                        path: None,
                        source_file: format!("{}:0", file.relative_path),
                        confidence: Confidence::High,
                        extraction_method: "compose".to_string(),
                        dependency: None,
                        evidence: Some(format!("{} depends_on {}", service_name, dep_name)),
                        ..Default::default()
                    });
                }
            }

            // Build ConnectionInfo for URL-valued environment: entries
            for (service_name, service) in &compose.services {
                let pairs: Vec<(String, String)> = match &service.environment {
                    ComposeEnv::None => vec![],
                    ComposeEnv::List(seq) => seq
                        .iter()
                        .filter_map(|item| {
                            let eq = item.find('=')?;
                            let key = item[..eq].to_string();
                            let val = item[eq + 1..].to_string();
                            Some((key, val))
                        })
                        .collect(),
                    ComposeEnv::Map(m) => m
                        .iter()
                        .filter_map(|(k, v)| {
                            // Only process string values — numbers/booleans are not URLs
                            let val = v.as_str().map(|s| s.to_string())?;
                            Some((k.clone(), val))
                        })
                        .collect(),
                };

                for (key, val) in pairs {
                    if !is_connection_key(&key) {
                        continue;
                    }
                    if let Some((protocol, hostname)) = parse_url_value(&val) {
                        result.connections.push(ConnectionInfo {
                            source_service: service_name.clone(),
                            target_name: hostname,
                            protocol,
                            method: None,
                            path: None,
                            source_file: format!("{}:0", file.relative_path),
                            confidence: Confidence::High,
                            extraction_method: "compose".to_string(),
                            dependency: None,
                            evidence: Some(format!("{}={}", key, val)),
                            ..Default::default()
                        });
                    }
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

    #[test]
    fn test_compose_with_dependencies() {
        let plugin = ComposePlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
version: "3.8"
services:
  api:
    build: .
    ports:
      - "3000:3000"
    environment:
      DB_HOST: db
      DB_PORT: 5432
    depends_on:
      - db
  db:
    image: postgres:15
    environment:
      POSTGRES_DB: myapp
"#;

        let file = FileContext {
            path: PathBuf::from("/repo/docker-compose.yml"),
            relative_path: "docker-compose.yml".to_string(),
            content: Arc::from(yaml_content),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        // Should have 2 services (api, db)
        assert_eq!(result.services.len(), 2);
        let service_names: std::collections::HashSet<_> =
            result.services.iter().map(|s| s.name.as_str()).collect();
        assert!(service_names.contains("api"));
        assert!(service_names.contains("db"));
        assert!(result
            .services
            .iter()
            .all(|s| s.confidence == Confidence::High));
        assert!(result
            .services
            .iter()
            .all(|s| s.extraction_method == "compose"));

        // Should have 1 connection (api -> db)
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].source_service, "api");
        assert_eq!(result.connections[0].target_name, "db");
        assert_eq!(result.connections[0].protocol, "compose-depends_on");
        assert_eq!(
            result.connections[0].evidence,
            Some("api depends_on db".to_string())
        );
    }

    #[test]
    fn test_compose_invalid_yaml() {
        let plugin = ComposePlugin;
        let root = PathBuf::from("/repo");
        let invalid_yaml = "not: valid: yaml: : :";

        let file = FileContext {
            path: PathBuf::from("/repo/docker-compose.yml"),
            relative_path: "docker-compose.yml".to_string(),
            content: Arc::from(invalid_yaml),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        // Should return empty result without panicking
        assert_eq!(result.services.len(), 0);
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_compose_with_map_depends_on() {
        let plugin = ComposePlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
version: "3.8"
services:
  web:
    image: myapp
    depends_on:
      db:
        condition: service_healthy
      cache:
        condition: service_started
  db:
    image: postgres:15
  cache:
    image: redis:7
"#;

        let file = FileContext {
            path: PathBuf::from("/repo/docker-compose.yml"),
            relative_path: "docker-compose.yml".to_string(),
            content: Arc::from(yaml_content),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        // Should have 3 services
        assert_eq!(result.services.len(), 3);

        // Should have 2 connections (web -> db, web -> cache)
        assert_eq!(result.connections.len(), 2);
        let targets: Vec<_> = result
            .connections
            .iter()
            .map(|c| c.target_name.as_str())
            .collect();
        assert!(targets.contains(&"db"));
        assert!(targets.contains(&"cache"));
    }

    #[test]
    fn test_file_patterns() {
        let plugin = ComposePlugin;
        let patterns = plugin.file_patterns();
        assert!(patterns.contains(&"**/docker-compose*.yml"));
        assert!(patterns.contains(&"**/docker-compose*.yaml"));
        assert!(patterns.contains(&"**/compose*.yml"));
        assert!(patterns.contains(&"**/compose*.yaml"));
    }

    #[test]
    fn test_compose_env_list_form() {
        let plugin = ComposePlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
version: "3.8"
services:
  api:
    image: myapp
    environment:
      - DATABASE_URL=postgres://db.host/mydb
"#;
        let file = FileContext {
            path: PathBuf::from("/repo/docker-compose.yml"),
            relative_path: "docker-compose.yml".to_string(),
            content: Arc::from(yaml_content),
        };
        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        let env_conns: Vec<_> = result
            .connections
            .iter()
            .filter(|c| c.extraction_method == "compose" && c.protocol != "compose-depends_on")
            .collect();
        assert_eq!(env_conns.len(), 1);
        assert_eq!(env_conns[0].source_service, "api");
        assert_eq!(env_conns[0].protocol, "postgresql");
        assert_eq!(env_conns[0].target_name, "db.host");
        assert_eq!(env_conns[0].extraction_method, "compose");
    }

    #[test]
    fn test_compose_env_map_form() {
        let plugin = ComposePlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
version: "3.8"
services:
  worker:
    image: myworker
    environment:
      REDIS_URL: "redis://cache.internal:6379"
"#;
        let file = FileContext {
            path: PathBuf::from("/repo/docker-compose.yml"),
            relative_path: "docker-compose.yml".to_string(),
            content: Arc::from(yaml_content),
        };
        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        let env_conns: Vec<_> = result
            .connections
            .iter()
            .filter(|c| c.protocol != "compose-depends_on")
            .collect();
        assert_eq!(env_conns.len(), 1);
        assert_eq!(env_conns[0].source_service, "worker");
        assert_eq!(env_conns[0].protocol, "redis");
        assert_eq!(env_conns[0].target_name, "cache.internal");
    }

    #[test]
    fn test_compose_env_non_url_skipped() {
        let plugin = ComposePlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
version: "3.8"
services:
  api:
    image: myapp
    environment:
      APP_NAME: myapp
      DB_PORT: 5432
"#;
        let file = FileContext {
            path: PathBuf::from("/repo/docker-compose.yml"),
            relative_path: "docker-compose.yml".to_string(),
            content: Arc::from(yaml_content),
        };
        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        let env_conns: Vec<_> = result
            .connections
            .iter()
            .filter(|c| c.protocol != "compose-depends_on")
            .collect();
        assert_eq!(env_conns.len(), 0);
    }

    #[test]
    fn test_compose_env_preserves_service_name() {
        let plugin = ComposePlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
version: "3.8"
services:
  frontend:
    image: frontend
    environment:
      DATABASE_URL: "postgres://db1.host/frontend"
  backend:
    image: backend
    environment:
      DATABASE_URL: "postgres://db2.host/backend"
"#;
        let file = FileContext {
            path: PathBuf::from("/repo/docker-compose.yml"),
            relative_path: "docker-compose.yml".to_string(),
            content: Arc::from(yaml_content),
        };
        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        let env_conns: Vec<_> = result
            .connections
            .iter()
            .filter(|c| c.protocol != "compose-depends_on")
            .collect();
        assert_eq!(env_conns.len(), 2);

        let service_names: std::collections::HashSet<_> = env_conns
            .iter()
            .map(|c| c.source_service.as_str())
            .collect();
        assert!(service_names.contains("frontend"));
        assert!(service_names.contains("backend"));

        let hosts: std::collections::HashSet<_> =
            env_conns.iter().map(|c| c.target_name.as_str()).collect();
        assert!(hosts.contains("db1.host"));
        assert!(hosts.contains("db2.host"));
    }
}
