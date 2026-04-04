use std::collections::HashMap;

use serde::Deserialize;
use tracing::warn;

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, ExtractionResult, ServiceInfo};

/// Docker Compose manifest parser.
pub struct ComposePlugin;

#[derive(Deserialize, Default)]
struct ComposeFile {
    #[serde(default)]
    services: HashMap<String, ComposeService>,
}

#[derive(Deserialize, Default)]
struct ComposeService {
    #[serde(default)]
    depends_on: DependsOn,
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
                        evidence: Some(format!("{} depends_on {}", service_name, dep_name)),
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
}
