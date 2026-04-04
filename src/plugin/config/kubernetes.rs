use serde::Deserialize;
use tracing::warn;

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ExtractionResult, ServiceInfo};

/// Kubernetes manifest parser (supports multi-document YAML).
pub struct KubernetesPlugin;

#[derive(Deserialize)]
struct K8sManifest {
    kind: Option<String>,
    metadata: Option<K8sMetadata>,
}

#[derive(Deserialize)]
struct K8sMetadata {
    name: Option<String>,
}

impl LanguagePlugin for KubernetesPlugin {
    fn name(&self) -> &str {
        "kubernetes"
    }

    fn file_patterns(&self) -> &[&str] {
        &[
            "**/k8s/**/*.yml",
            "**/k8s/**/*.yaml",
            "**/kubernetes/**/*.yml",
            "**/kubernetes/**/*.yaml",
            "**/manifests/**/*.yml",
            "**/manifests/**/*.yaml",
            "**/*.k8s.yml",
            "**/*.k8s.yaml",
        ]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        for file in &ctx.files {
            // Use multi-document YAML iterator
            let deserializer = serde_yaml_bw::Deserializer::from_str(&file.content);

            for document in deserializer {
                match K8sManifest::deserialize(document) {
                    Ok(manifest) => {
                        // Process based on kind
                        let kind = manifest.kind.as_deref().unwrap_or("");
                        match kind {
                            "Deployment" | "Service" => {
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
                                            extraction_method: "kubernetes".to_string(),
                                        });
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
                        warn!(
                            "kubernetes: failed to parse document in {}: {}",
                            file.relative_path, e
                        );
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
    fn test_kubernetes_deployment() {
        let plugin = KubernetesPlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api-server
  namespace: default
spec:
  replicas: 2
  selector:
    matchLabels:
      app: api-server
  template:
    metadata:
      labels:
        app: api-server
    spec:
      containers:
        - name: api
          image: my-org/api:latest
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
        };

        let result = plugin.extract(&ctx);

        // Should have 1 service (api-server)
        assert_eq!(result.services.len(), 1);
        assert_eq!(result.services[0].name, "api-server");
        assert_eq!(result.services[0].root_path, "k8s");
        assert_eq!(result.services[0].confidence, Confidence::High);
        assert_eq!(result.services[0].extraction_method, "kubernetes");
    }

    #[test]
    fn test_kubernetes_configmap_skipped() {
        let plugin = KubernetesPlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
  namespace: default
data:
  DB_HOST: "postgres-service"
  DB_PORT: "5432"
"#;

        let file = FileContext {
            path: PathBuf::from("/repo/k8s/configmap.yaml"),
            relative_path: "k8s/configmap.yaml".to_string(),
            content: Arc::from(yaml_content),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
        };

        let result = plugin.extract(&ctx);

        // ConfigMap should not produce ServiceInfo
        assert_eq!(result.services.len(), 0);
    }

    #[test]
    fn test_kubernetes_multi_document() {
        let plugin = KubernetesPlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: api-server
spec:
  replicas: 1
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
data:
  DB_HOST: "db-service"
---
apiVersion: v1
kind: Service
metadata:
  name: api-service
spec:
  type: ClusterIP
"#;

        let file = FileContext {
            path: PathBuf::from("/repo/k8s/manifest.yaml"),
            relative_path: "k8s/manifest.yaml".to_string(),
            content: Arc::from(yaml_content),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
        };

        let result = plugin.extract(&ctx);

        // Should have 2 services (Deployment + Service, ConfigMap skipped)
        assert_eq!(result.services.len(), 2);
        let service_names: std::collections::HashSet<_> =
            result.services.iter().map(|s| s.name.as_str()).collect();
        assert!(service_names.contains("api-server"));
        assert!(service_names.contains("api-service"));
    }

    #[test]
    fn test_file_patterns() {
        let plugin = KubernetesPlugin;
        let patterns = plugin.file_patterns();
        assert!(patterns.contains(&"**/k8s/**/*.yml"));
        assert!(patterns.contains(&"**/k8s/**/*.yaml"));
        assert!(patterns.contains(&"**/kubernetes/**/*.yml"));
        assert!(patterns.contains(&"**/*.k8s.yaml"));
    }
}
