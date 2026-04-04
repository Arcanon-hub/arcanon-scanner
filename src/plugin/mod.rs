//! Plugin host: LanguagePlugin trait and compiled-in registry.
//!
//! # Panic Safety (FTOL-02)
//! The scanner calls `plugin.extract()` via `std::panic::catch_unwind(AssertUnwindSafe(...))`.
//! If a plugin panics, the panic is caught and logged; other plugins continue normally.
//! Plugin authors: do not rely on panics for control flow.
//!
//! # Sync Boundary (PITFALLS.md Pitfall 4)
//! All plugins are SYNCHRONOUS. The `extract()` method MUST NOT use async/await or import tokio.
//! Plugins run on rayon's thread pool. Async work (upload) runs separately on tokio.
//! Calling tokio from a rayon thread causes deadlocks.

// HARD BOUNDARY: NO TOKIO IMPORTS IN THIS DIRECTORY OR ANY SUBDIRECTORY.
// All plugin code runs on rayon threads (CPU-bound). Calling .await or
// tokio::block_on() from a rayon thread causes deadlocks (PITFALLS.md Pitfall 4).
// The only async code is in src/upload/mod.rs.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::types::ExtractionResult;
use crate::vars::VariableStore;

pub mod config;
pub mod lang;

/// A single file's content and path metadata, passed to plugins.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FileContext {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// Path relative to repo root (used for service scoping).
    pub relative_path: String,
    /// File contents — read once and shared across all plugins.
    pub content: Arc<str>,
}

/// All inputs available to a plugin during extraction.
#[allow(dead_code)]
pub struct ExtractionContext {
    /// All files matching this plugin's file_patterns.
    pub files: Vec<FileContext>,
    /// Variable resolution store (.env, compose, k8s values).
    pub vars: Arc<VariableStore>,
    /// Absolute path to the repo root.
    pub root: PathBuf,
    /// Absolute paths of service roots → service name.
    /// Built from config-plugin ServiceInfo before language plugins run (Phase 4 addition).
    /// Empty HashMap means monorepo scoping is not applicable (single-service repo).
    pub service_roots: HashMap<PathBuf, String>,
}

/// Returns the nearest-ancestor service name for a given file path.
/// Walks file_path.ancestors() from most-specific to least-specific.
/// Returns None if the file is not under any known service root (unscoped).
///
/// # Arguments
/// * `file_path` - Absolute path to a file
/// * `service_roots` - Map of service root paths to service names
///
/// # Returns
/// Option<&str> with the service name if found, None if unscoped
pub fn scope_to_service<'a>(
    file_path: &std::path::Path,
    service_roots: &'a HashMap<PathBuf, String>,
) -> Option<&'a str> {
    file_path
        .ancestors()
        .find_map(|ancestor| service_roots.get(ancestor))
        .map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_to_service_exact_match() {
        let mut service_roots = HashMap::new();
        service_roots.insert(PathBuf::from("/repo/service-a"), "service-a".to_string());

        let file_path = PathBuf::from("/repo/service-a/src/app.ts");
        let result = scope_to_service(&file_path, &service_roots);

        assert_eq!(result, Some("service-a"));
    }

    #[test]
    fn test_scope_to_service_nested_file() {
        let mut service_roots = HashMap::new();
        service_roots.insert(PathBuf::from("/repo/service-b"), "service-b".to_string());

        let file_path = PathBuf::from("/repo/service-b/src/handlers/user.ts");
        let result = scope_to_service(&file_path, &service_roots);

        assert_eq!(result, Some("service-b"));
    }

    #[test]
    fn test_scope_to_service_unscoped_file() {
        let mut service_roots = HashMap::new();
        service_roots.insert(PathBuf::from("/repo/service-a"), "service-a".to_string());

        let file_path = PathBuf::from("/repo/lib/shared.ts");
        let result = scope_to_service(&file_path, &service_roots);

        assert_eq!(result, None);
    }

    #[test]
    fn test_scope_to_service_nearest_ancestor() {
        let mut service_roots = HashMap::new();
        service_roots.insert(PathBuf::from("/repo/service-c"), "service-c".to_string());
        service_roots.insert(
            PathBuf::from("/repo/service-c/submodule"),
            "submodule".to_string(),
        );

        let file_path = PathBuf::from("/repo/service-c/submodule/index.ts");
        let result = scope_to_service(&file_path, &service_roots);

        // Should find the nearest (most specific) ancestor
        assert_eq!(result, Some("submodule"));
    }

    #[test]
    fn test_scope_to_service_empty_map() {
        let service_roots = HashMap::new();

        let file_path = PathBuf::from("/repo/service-a/src/app.ts");
        let result = scope_to_service(&file_path, &service_roots);

        assert_eq!(result, None);
    }
}

/// The plugin trait that all 15 built-in plugins (8 config + 7 language) implement.
///
/// extract() is SYNCHRONOUS. Do not add async methods. Do not use tokio inside implementations.
#[allow(dead_code)]
pub trait LanguagePlugin: Send + Sync {
    /// Human-readable plugin name (e.g., "typescript", "openapi").
    fn name(&self) -> &str;

    /// Glob patterns this plugin wants to receive.
    /// Config plugins: ["**/openapi.{json,yaml,yml}"]
    /// Language plugins: ["**/*.ts", "**/*.tsx"]
    fn file_patterns(&self) -> &[&str];

    /// Whether this plugin runs even when no files match (true for config plugins).
    fn always_run(&self) -> bool {
        false
    }

    /// Extract findings from matched files. Returns empty ExtractionResult if
    /// no findings. Must not panic — callers catch panics for fault tolerance.
    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult;
}

/// Build the default plugin registry. All 15 plugins compiled in.
/// Stubs in Phase 1 — config plugins implemented in Phase 3, language plugins in Phase 4.
pub fn default_plugins() -> Vec<Box<dyn LanguagePlugin>> {
    vec![
        // Config plugins (always run)
        Box::new(config::OpenApiPlugin),
        Box::new(config::ProtoPlugin),
        Box::new(config::GraphqlPlugin),
        Box::new(config::AsyncApiPlugin),
        Box::new(config::ComposePlugin),
        Box::new(config::KubernetesPlugin),
        Box::new(config::DockerfilePlugin),
        Box::new(config::EnvPlugin),
        // Language plugins (run when files match)
        Box::new(lang::TypeScriptPlugin),
        Box::new(lang::PythonPlugin),
        Box::new(lang::GoPlugin),
        Box::new(lang::JavaPlugin),
        Box::new(lang::CSharpPlugin),
        Box::new(lang::RustLangPlugin),
        Box::new(lang::RubyPlugin),
    ]
}
