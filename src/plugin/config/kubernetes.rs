//! Kubernetes manifest extraction plugin.
//! Extracts service names from Deployments/Services and connections from container environment variables.

use crate::types::{Confidence, ConnectionInfo, ExtractionResult, ServiceInfo};
use crate::plugin::{LanguagePlugin, ExtractionContext};
use serde::Deserialize;
use tracing::warn;

/// A partial Kubernetes manifest for extraction.
#[derive(Debug, Deserialize)]
struct K8sManifest {
    kind: String,
    metadata: Option<K8sMetadata>,
    spec: Option<K8sSpec>,
}

#[derive(Debug, Deserialize)]
struct K8sMetadata {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct K8sSpec {
    template: Option<PodTemplate>,
}

#[derive(Debug, Deserialize)]
struct PodTemplate {
    spec: Option<PodSpec>,
}

#[derive(Debug, Deserialize)]
struct PodSpec {
    containers: Vec<Container>,
}

#[derive(Debug, Deserialize)]
struct Container {
    env: Vec<EnvVar>,
}

#[derive(Debug, Deserialize)]
struct EnvVar {
    name: String,
    value: Option<String>,
}

/// The Kubernetes extraction plugin.
pub struct KubernetesPlugin;

impl LanguagePlugin for KubernetesPlugin {
    fn name(&self) -> &str {
        "kubernetes"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.yaml", "**/*.yml"]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        for file in &ctx.files {
            // Only process files that look like K8s manifests
            if !file.content.contains("apiVersion:") || !file.content.contains("kind:") {
                continue;
            }

            // A single file might contain multiple YAML documents separated by ---
            for doc in file.content.split("---") {
                let trimmed = doc.trim();
                if trimmed.is_empty() {
                    continue;
                }

                match serde_yaml_bw::from_str::<K8sManifest>(trimmed) {
                    Ok(manifest) => {
                        match manifest.kind.as_str() {
                            "Deployment" | "StatefulSet" | "DaemonSet" | "Job" => {
                                if let Some(metadata) = manifest.metadata {
                                    if let Some(name) = metadata.name {
                                        // Derive root_path: parent directory of the k8s file
                                        let root_path = if let Some(parent) = file.path.parent() {
                                            if let Ok(rel) = parent.strip_prefix(&ctx.root) {
                                                rel.to_string_lossy().to_string()
                                            } else {
                                                String::new()
                                            }
                                        } else {
                                            String::new()
                                        };

                                        result.services.push(ServiceInfo {
                                            name: name.clone(),
                                            root_path,
                                            language: String::new(),
                                            service_type: "service".to_string(),
                                            boundary_entry: None,
                                            confidence: Confidence::High,
                                            extraction_method: "kubernetes".to_string()
                                        });

                                        // Extract connections from containers[].env
                                        if let Some(spec) = manifest.spec {
                                            if let Some(template) = spec.template {
                                                if let Some(pod_spec) = template.spec {
                                                    for container in pod_spec.containers {
                                                        for env_var in container.env {
                                                            if let Some(val) = env_var.value {
                                                                if is_connection_key(&env_var.name) {
                                                                    let (protocol, hostname) = if let Some(parsed) = parse_url_value(&val) {
                                                                        parsed
                                                                    } else if looks_like_hostname(&val) {
                                                                        // Plain hostname without scheme — infer protocol from key name
                                                                        let proto = infer_protocol_from_key(&env_var.name);
                                                                        (proto, val.clone())
                                                                    } else {
                                                                        continue;
                                                                    };
                                                                    result.connections.push(ConnectionInfo {
                                                                        source_service: name.clone(),
                                                                        target_name: hostname,
                                                                        protocol,
                                                                        method: None,
                                                                        path: None,
                                                                        source_file: format!("{}:0", file.relative_path),
                                                                        confidence: Confidence::High,
                                                                        extraction_method: "kubernetes".to_string(),
                                                                        dependency: None,
                                                                        evidence: Some(format!("{}={}", env_var.name, val)),
                                                                        ml_confidence: None,
                                                                        ml_reasoning: None,
                                                                    });
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            "Service" => {
                                if let Some(metadata) = manifest.metadata {
                                    if let Some(name) = metadata.name {
                                        // Derive root_path: parent directory of the k8s file
                                        let root_path = if let Some(parent) = file.path.parent() {
                                            if let Ok(rel) = parent.strip_prefix(&ctx.root) {
                                                rel.to_string_lossy().to_string()
                                            } else {
                                                String::new()
                                            }
                                        } else {
                                            String::new()
                                        };

                                        result.services.push(ServiceInfo {
                                            name,
                                            root_path,
                                            language: String::new(),
                                            service_type: "service".to_string(),
                                            boundary_entry: None,
                                            confidence: Confidence::High,
                                            extraction_method: "kubernetes".to_string()
                                        });
                                        // no container env traversal for Service kind
                                    }
                                }
                            }
                            "ConfigMap" => {
                                // Skip ConfigMap — VariableStore extracts these
                            }
                            _ => {
                                // Skip other kinds silently
                            }
                        }
                    }
                    Err(e) => {
                        warn!("kubernetes: failed to parse document in {}: {}", file.relative_path, e);
                    }
                }
            }
        }

        result
    }
}

/// Basic heuristic for environment variables that might contain service connection URLs.
fn is_connection_key(name: &str) -> bool {
    let n = name.to_uppercase();
    n.contains("URL") || n.contains("HOST") || n.contains("ENDPOINT") || n.contains("BROKER") || n.contains("ADDRESS")
}

/// Simple URL parser for environment variable values.
fn parse_url_value(val: &str) -> Option<(String, String)> {
    if let Some(pos) = val.find("://") {
        let protocol = val[..pos].to_string();
        let rest = &val[pos + 3..];

        // Find end of hostname (next / or : or end of string)
        let end = rest.find(['/', ':']).unwrap_or(rest.len());
        let hostname = rest[..end].to_string();

        if !hostname.is_empty() {
            return Some((protocol, hostname));
        }
    }
    None
}

/// Returns true if `val` looks like a plain hostname (e.g. "redis.cache", "db.internal").
/// Must contain a dot and no spaces. Does NOT match bare words like "localhost".
fn looks_like_hostname(val: &str) -> bool {
    !val.is_empty()
        && !val.contains(' ')
        && val.contains('.')
        && !val.contains("://")
}

/// Infer a protocol string from the environment variable key name.
fn infer_protocol_from_key(name: &str) -> String {
    let n = name.to_uppercase();
    if n.contains("REDIS") {
        "redis".to_string()
    } else if n.contains("POSTGRES") || n.contains("PG") || n.contains("DATABASE") {
        "postgresql".to_string()
    } else if n.contains("MYSQL") {
        "mysql".to_string()
    } else if n.contains("MONGO") {
        "mongodb".to_string()
    } else if n.contains("KAFKA") || n.contains("BROKER") {
        "kafka".to_string()
    } else {
        "tcp".to_string()
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
    fn test_kubernetes_deployment_extraction() {
        let plugin = KubernetesPlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api-server
spec:
  template:
    spec:
      containers:
        - name: api
          env:
            - name: DATABASE_URL
              value: postgres://db.internal:5432/myapp
            - name: REDIS_HOST
              value: redis.cache
"#;

        let file = FileContext {
            path: PathBuf::from("/repo/k8s/deployment.yaml"),
            relative_path: "k8s/deployment.yaml".to_string(),
            content: Arc::from(yaml_content),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        // Should find 1 service
        assert_eq!(result.services.len(), 1);
        assert_eq!(result.services[0].name, "api-server");

        // Should find 2 connections (DB and Redis)
        assert_eq!(result.connections.len(), 2);
        let protocols: std::collections::HashSet<_> = result.connections.iter().map(|c| c.protocol.as_str()).collect();
        assert!(protocols.contains("postgres"));
        assert!(result.connections.iter().any(|c| c.target_name == "redis.cache"));
    }

    #[test]
    fn test_kubernetes_service_extraction() {
        let plugin = KubernetesPlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
apiVersion: v1
kind: Service
metadata:
  name: user-service
spec:
  ports:
    - port: 80
      targetPort: 8080
"#;

        let file = FileContext {
            path: PathBuf::from("/repo/k8s/service.yaml"),
            relative_path: "k8s/service.yaml".to_string(),
            content: Arc::from(yaml_content),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert_eq!(result.services.len(), 1);
        assert_eq!(result.services[0].name, "user-service");
    }

    #[test]
    fn test_is_connection_key() {
        assert!(is_connection_key("DATABASE_URL"));
        assert!(is_connection_key("REDIS_HOST"));
        assert!(is_connection_key("KAFKA_BROKERS"));
        assert!(!is_connection_key("APP_NAME"));
        assert!(!is_connection_key("PORT"));
    }

    #[test]
    fn test_parse_url_value() {
        assert_eq!(parse_url_value("postgres://db:5432"), Some(("postgres".to_string(), "db".to_string())));
        assert_eq!(parse_url_value("http://api.internal/v1"), Some(("http".to_string(), "api.internal".to_string())));
        assert_eq!(parse_url_value("not-a-url"), None);
    }
}
