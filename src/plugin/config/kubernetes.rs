use serde::Deserialize;
use tracing::warn;

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, ExtractionResult, ServiceInfo};

use super::url_util::{is_connection_key, parse_url_value};

/// Kubernetes manifest parser (supports multi-document YAML).
pub struct KubernetesPlugin;

#[derive(Deserialize)]
struct K8sManifest {
    kind: Option<String>,
    metadata: Option<K8sMetadata>,
    spec: Option<K8sSpec>,
}

#[derive(Deserialize)]
struct K8sMetadata {
    name: Option<String>,
}

#[derive(Deserialize)]
struct K8sSpec {
    template: Option<K8sTemplate>,
}

#[derive(Deserialize)]
struct K8sTemplate {
    spec: Option<K8sPodSpec>,
}

#[derive(Deserialize)]
struct K8sPodSpec {
    #[serde(default)]
    containers: Vec<K8sContainer>,
}

#[derive(Deserialize)]
struct K8sContainer {
    #[serde(default)]
    env: Vec<K8sEnvVar>,
}

#[derive(Deserialize)]
struct K8sEnvVar {
    name: String,
    value: Option<String>, // None when valueFrom is used; serde ignores valueFrom by default
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
                            "Deployment" => {
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
                                            extraction_method: "kubernetes".to_string(),
                                        });

                                        // Extract connections from containers[].env
                                        if let Some(spec) = manifest.spec {
                                            if let Some(template) = spec.template {
                                                if let Some(pod_spec) = template.spec {
                                                    for container in pod_spec.containers {
                                                        for env_var in container.env {
                                                            if let Some(val) = env_var.value {
                                                                if is_connection_key(&env_var.name)
                                                                {
                                                                    if let Some((
                                                                        protocol,
                                                                        hostname,
                                                                    )) = parse_url_value(&val)
                                                                    {
                                                                        result
                                                                            .connections
                                                                            .push(ConnectionInfo {
                                                                            source_service: name
                                                                                .clone(),
                                                                            target_name: hostname,
                                                                            protocol,
                                                                            method: None,
                                                                            path: None,
                                                                            source_file: format!(
                                                                                "{}:0",
                                                                                file.relative_path
                                                                            ),
                                                                            confidence:
                                                                                Confidence::High,
                                                                            extraction_method:
                                                                                "kubernetes"
                                                                                    .to_string(),
                                                                            dependency: None,
                                                                            evidence: Some(
                                                                                format!(
                                                                                    "{}={}",
                                                                                    env_var.name,
                                                                                    val
                                                                                ),
                                                                            ),
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
                                            extraction_method: "kubernetes".to_string(),
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
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        // Should have 1 service (api-server)
        assert_eq!(result.services.len(), 1);
        assert_eq!(result.services[0].name, "api-server");
        assert_eq!(result.services[0].root_path, "k8s");
        assert_eq!(result.services[0].confidence, Confidence::High);
        assert_eq!(result.services[0].extraction_method, "kubernetes");
        // No env entries so no connections
        assert_eq!(result.connections.len(), 0);
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
            service_roots: std::collections::HashMap::new(),
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
            service_roots: std::collections::HashMap::new(),
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

    #[test]
    fn test_k8s_deployment_env_url() {
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
          image: my-org/api:latest
          env:
            - name: DATABASE_URL
              value: "postgres://db.host/mydb"
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

        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.source_service, "api-server");
        assert_eq!(conn.protocol, "postgresql");
        assert_eq!(conn.target_name, "db.host");
        assert_eq!(conn.extraction_method, "kubernetes");
        assert_eq!(conn.confidence, Confidence::High);
    }

    #[test]
    fn test_k8s_deployment_env_value_from_skipped() {
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
          image: my-org/api:latest
          env:
            - name: DATABASE_URL
              valueFrom:
                secretKeyRef:
                  name: db-secret
                  key: url
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

        // valueFrom has no literal value field — should produce 0 connections
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_k8s_deployment_env_non_url_skipped() {
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
          image: my-org/api:latest
          env:
            - name: APP_NAME
              value: "myapp"
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

        // APP_NAME does not match connection key pattern
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_k8s_deployment_env_multiple_containers() {
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
          image: my-org/api:latest
          env:
            - name: DATABASE_URL
              value: "postgres://db.host/mydb"
        - name: sidecar
          image: my-org/sidecar:latest
          env:
            - name: DATABASE_URL
              value: "postgres://db.host/mydb"
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

        // 2 containers, each with DATABASE_URL → 2 connections, both with deployment name
        assert_eq!(result.connections.len(), 2);
        for conn in &result.connections {
            assert_eq!(conn.source_service, "api-server");
        }
    }

    #[test]
    fn test_k8s_service_kind_no_env() {
        let plugin = KubernetesPlugin;
        let root = PathBuf::from("/repo");
        let yaml_content = r#"
apiVersion: v1
kind: Service
metadata:
  name: api-service
spec:
  selector:
    app: api-server
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

        // Service kind should emit ServiceInfo but NO connections
        assert_eq!(result.services.len(), 1);
        assert_eq!(result.services[0].name, "api-service");
        assert_eq!(result.connections.len(), 0);
    }

    #[test]
    fn test_k8s_multi_doc_with_env() {
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
              value: "postgres://db.host/mydb"
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
data:
  key: value
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
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        // 1 connection from Deployment, 0 from ConfigMap
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].source_service, "api-server");
    }
}
