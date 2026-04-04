use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ExtractionResult, ServiceInfo};

/// Dockerfile/Containerfile parser — detects service boundaries.
pub struct DockerfilePlugin;

impl LanguagePlugin for DockerfilePlugin {
    fn name(&self) -> &str {
        "dockerfile"
    }

    fn file_patterns(&self) -> &[&str] {
        &[
            "**/Dockerfile",
            "**/Dockerfile.*",
            "**/Containerfile",
            "**/Containerfile.*",
        ]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut services = Vec::new();

        for file in &ctx.files {
            // Get parent directory (service root)
            if let Some(parent) = file.path.parent() {
                // Compute root_path: relative path from ctx.root to parent
                let root_path = if let Ok(rel) = parent.strip_prefix(&ctx.root) {
                    rel.to_string_lossy().to_string()
                } else {
                    String::new()
                };

                // Service name: basename of parent directory
                let service_name = parent
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "root".to_string());

                services.push(ServiceInfo {
                    name: service_name,
                    root_path,
                    language: String::new(),
                    service_type: "service".to_string(),
                    boundary_entry: Some(file.relative_path.clone()),
                    confidence: Confidence::High,
                    extraction_method: "dockerfile".to_string(),
                });
            }
        }

        ExtractionResult {
            services,
            ..Default::default()
        }
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
    fn test_dockerfile_at_root() {
        let plugin = DockerfilePlugin;
        let root = PathBuf::from("/repo");
        let file = FileContext {
            path: PathBuf::from("/repo/Dockerfile"),
            relative_path: "Dockerfile".to_string(),
            content: Arc::from("FROM node:20"),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);
        assert_eq!(result.services.len(), 1);
        assert_eq!(result.services[0].name, "repo");
        assert_eq!(result.services[0].root_path, "");
        assert_eq!(result.services[0].confidence, Confidence::High);
        assert_eq!(result.services[0].extraction_method, "dockerfile");
    }

    #[test]
    fn test_dockerfile_in_subdirectory() {
        let plugin = DockerfilePlugin;
        let root = PathBuf::from("/repo");
        let file = FileContext {
            path: PathBuf::from("/repo/services/api/Dockerfile"),
            relative_path: "services/api/Dockerfile".to_string(),
            content: Arc::from("FROM node:20"),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);
        assert_eq!(result.services.len(), 1);
        assert_eq!(result.services[0].name, "api");
        assert_eq!(result.services[0].root_path, "services/api");
        assert_eq!(
            result.services[0].boundary_entry,
            Some("services/api/Dockerfile".to_string())
        );
    }

    #[test]
    fn test_multiple_dockerfiles() {
        let plugin = DockerfilePlugin;
        let root = PathBuf::from("/repo");
        let files = vec![
            FileContext {
                path: PathBuf::from("/repo/services/api/Dockerfile"),
                relative_path: "services/api/Dockerfile".to_string(),
                content: Arc::from("FROM node:20"),
            },
            FileContext {
                path: PathBuf::from("/repo/services/db/Dockerfile"),
                relative_path: "services/db/Dockerfile".to_string(),
                content: Arc::from("FROM postgres:15"),
            },
        ];

        let ctx = ExtractionContext {
            files,
            vars: Arc::new(VariableStore::new()),
            root,
            service_roots: std::collections::HashMap::new(),
        };

        let result = plugin.extract(&ctx);
        assert_eq!(result.services.len(), 2);
        assert_eq!(result.services[0].name, "api");
        assert_eq!(result.services[1].name, "db");
    }

    #[test]
    fn test_file_patterns() {
        let plugin = DockerfilePlugin;
        let patterns = plugin.file_patterns();
        assert!(patterns.contains(&"**/Dockerfile"));
        assert!(patterns.contains(&"**/Dockerfile.*"));
        assert!(patterns.contains(&"**/Containerfile"));
        assert!(patterns.contains(&"**/Containerfile.*"));
    }
}
