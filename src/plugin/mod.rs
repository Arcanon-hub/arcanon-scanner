// HARD BOUNDARY: NO TOKIO IMPORTS IN THIS DIRECTORY OR ANY SUBDIRECTORY.
// All plugin code runs on rayon threads (CPU-bound). Calling .await or
// tokio::block_on() from a rayon thread causes deadlocks (PITFALLS.md Pitfall 4).
// The only async code is in src/upload/mod.rs.

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
