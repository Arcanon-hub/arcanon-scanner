//! Orchestration pipeline for the scanner.
//! Wires all components: file discovery → variable store → git context → parallel plugin execution
//! → merge → resolve → payload assembly → upload/output/dry-run.
//!
//! Implements PIPE-05 (rayon parallel execution) and FTOL-02 (panic isolation per plugin).

use crate::core::{merger, payload, resolver};
use crate::discovery::walk_repo;
use crate::git::detect_git_context;
use crate::plugin::{default_plugins, ExtractionContext, FileContext, LanguagePlugin};
use crate::types::ExtractionResult;
use crate::vars::build_variable_store;
use anyhow::Result;
use globset::GlobSetBuilder;
use rayon::prelude::*;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Configuration for a scanner run.
#[derive(Debug, Clone)]
pub struct ScannerConfig {
    /// Root directory to scan
    pub root: PathBuf,
    /// Print payload to stdout and exit without uploading
    pub dry_run: bool,
    /// Write payload to this file instead of uploading
    pub output: Option<PathBuf>,
    /// Hub URL for upload (or stub if --dry-run)
    pub hub_url: String,
    /// API key for upload (or stub if --dry-run)
    pub api_key: String,
    /// Project slug identifier
    pub project_slug: String,
    /// Plugin filter (comma-separated names to include; None = include all)
    pub plugin_filter: Option<String>,
    /// Additional exclude patterns
    pub exclude_patterns: Vec<String>,
    /// Service overrides from .arcanon.toml [services]
    pub service_overrides: HashMap<String, merger::ServiceOverride>,
    /// Git overrides from CLI or env vars
    pub git_overrides: GitOverrides,
}

/// Git-related CLI overrides.
#[derive(Debug, Clone, Default)]
pub struct GitOverrides {
    pub repo_url: Option<String>,
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
}

/// Run the scanner against a directory.
///
/// This is the main orchestration function. It:
/// 1. Records start time
/// 2. Detects git context
/// 3. Builds variable store
/// 4. Discovers files
/// 5. Filters plugins by name
/// 6. Runs plugins in parallel with panic isolation (rayon + catch_unwind)
/// 7. Merges results
/// 8. Checks for empty findings (warning)
/// 9. Applies service overrides
/// 10. Resolves intra-repo connections
/// 11. Assembles ScanPayloadV1
/// 12. Returns payload
pub fn run(config: &ScannerConfig) -> Result<payload::ScanPayloadV1> {
    info!("Scanner starting: {}", config.root.display());

    // Step 1: Record start time
    let started_at = now_rfc3339();
    debug!("scan started at {}", started_at);

    // Step 2: Detect git context and apply overrides
    let mut git_context = detect_git_context(&config.root)?;

    // Apply CLI/env var overrides (precedence: override > detected)
    if let Some(url) = &config.git_overrides.repo_url {
        git_context.repo_url = Some(url.clone());
    }
    if let Some(branch) = &config.git_overrides.branch {
        git_context.branch = branch.clone();
    }
    if let Some(sha) = &config.git_overrides.commit_sha {
        git_context.commit_sha = sha.clone();
    }

    debug!("git context: {:?}", git_context);

    // Step 3: Build variable store
    let all_files_for_vars = walk_repo(&config.root, &config.exclude_patterns)?;
    let vars = build_variable_store(&config.root, &all_files_for_vars);
    debug!("variable store built");

    // Step 4: Discover all files (converting PathBuf to FileContext)
    let all_files_paths = walk_repo(&config.root, &config.exclude_patterns)?;
    let mut all_files: Vec<FileContext> = Vec::new();
    for path in &all_files_paths {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let relative = path
                    .strip_prefix(&config.root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .to_string();
                all_files.push(FileContext {
                    path: path.clone(),
                    relative_path: relative,
                    content: Arc::from(content),
                });
            }
            Err(e) => {
                warn!("Failed to read {}: {}", path.display(), e);
                continue;
            }
        }
    }
    info!("Discovered {} files", all_files.len());

    // Step 5: Get all plugins
    let all_plugins = default_plugins();

    // Step 5b: Filter plugins if --plugins flag is set
    let plugins: Vec<Box<dyn LanguagePlugin>> = if let Some(ref filter) = config.plugin_filter {
        let names: Vec<&str> = filter.split(',').map(|s| s.trim()).collect();
        all_plugins
            .into_iter()
            .filter(|p| names.contains(&p.name()))
            .collect()
    } else {
        all_plugins
    };

    info!("Using {} plugins (filtered from all)", plugins.len());
    for p in &plugins {
        debug!("  - {}", p.name());
    }

    // Step 6: Run plugins in parallel
    let vars_arc = Arc::new(vars);
    let results = run_plugins_parallel(&plugins, &all_files, &vars_arc, &config.root);

    info!("Plugin execution complete: {} results", results.len());

    // Step 7: Merge all results
    let mut merged = merger::merge(results);
    debug!("Merged into {} services", merged.services.len());

    // Step 8: Check for empty findings (warning)
    merger::check_empty_findings(&merged);

    // Step 9: Apply service overrides
    merger::apply_service_overrides(&mut merged, &config.service_overrides);

    // Step 10: Resolve intra-repo connections
    let merged = resolver::resolve(merged);
    debug!("Resolved connections");

    // Step 11: Record end time and assemble payload
    let completed_at = now_rfc3339();
    let payload = payload::assemble(
        merged,
        git_context.repo_url.clone(),
        git_context.repo_name.clone(),
        git_context.branch.clone(),
        git_context.commit_sha.clone(),
        config.project_slug.clone(),
        started_at,
        completed_at,
        all_files.len(),
    );

    info!(
        "Payload assembled: {} services, {} endpoints, {} connections, {} schemas",
        payload.findings.services.len(),
        payload
            .findings
            .services
            .iter()
            .map(|s| s.exposes.len())
            .sum::<usize>(),
        payload.findings.connections.len(),
        payload.findings.schemas.len()
    );

    Ok(payload)
}

/// Run all plugins in parallel, applying filter by file_patterns.
/// Each plugin is wrapped in catch_unwind for fault tolerance (FTOL-02).
///
/// For each plugin:
/// - If always_run() is true, pass all matching files (even if empty)
/// - If always_run() is false and no files match, skip the plugin
fn run_plugins_parallel(
    plugins: &[Box<dyn LanguagePlugin>],
    all_files: &[FileContext],
    vars: &Arc<crate::vars::VariableStore>,
    root: &Path,
) -> Vec<ExtractionResult> {
    plugins
        .par_iter()
        .filter_map(|plugin| {
            debug!("Running plugin: {}", plugin.name());

            // Filter files matching this plugin's patterns
            let matching = filter_files_by_patterns(all_files, plugin.file_patterns());

            // Skip plugins that don't match any files and don't always run
            if matching.is_empty() && !plugin.always_run() {
                debug!("  skipped: no matching files");
                return None;
            }

            // Build extraction context
            let ctx = ExtractionContext {
                files: matching,
                vars: Arc::clone(vars),
                root: root.to_path_buf(),
            };

            // Execute plugin with panic isolation (FTOL-02)
            match panic::catch_unwind(AssertUnwindSafe(|| plugin.extract(&ctx))) {
                Ok(result) => {
                    debug!(
                        "  {} services, {} endpoints, {} connections, {} schemas",
                        result.services.len(),
                        result.endpoints.len(),
                        result.connections.len(),
                        result.schemas.len()
                    );
                    Some(result)
                }
                Err(panic_val) => {
                    let msg = panic_val
                        .downcast_ref::<&str>()
                        .copied()
                        .unwrap_or("unknown panic");
                    warn!("Plugin '{}' panicked: {}", plugin.name(), msg);
                    None
                }
            }
        })
        .collect()
}

/// Filter files by glob patterns using globset.
fn filter_files_by_patterns(files: &[FileContext], patterns: &[&str]) -> Vec<FileContext> {
    if patterns.is_empty() {
        return Vec::new();
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        if let Ok(glob) = globset::Glob::new(pattern) {
            builder.add(glob);
        }
    }

    let globset = match builder.build() {
        Ok(gs) => gs,
        Err(e) => {
            warn!("Failed to compile glob patterns: {}", e);
            return Vec::new();
        }
    };

    files
        .iter()
        .filter(|f| globset.is_match(&f.relative_path))
        .cloned()
        .collect()
}

/// Generate an RFC3339 timestamp for the current time.
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_files_by_patterns_single_match() {
        let files = vec![
            FileContext {
                path: PathBuf::from("openapi.yaml"),
                relative_path: "openapi.yaml".to_string(),
                content: Arc::from(""),
            },
            FileContext {
                path: PathBuf::from("src/main.ts"),
                relative_path: "src/main.ts".to_string(),
                content: Arc::from(""),
            },
        ];

        let matched = filter_files_by_patterns(&files, &["**/openapi.*"]);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].relative_path, "openapi.yaml");
    }

    #[test]
    fn test_filter_files_by_patterns_multiple_match() {
        let files = vec![
            FileContext {
                path: PathBuf::from("src/main.ts"),
                relative_path: "src/main.ts".to_string(),
                content: Arc::from(""),
            },
            FileContext {
                path: PathBuf::from("src/app.tsx"),
                relative_path: "src/app.tsx".to_string(),
                content: Arc::from(""),
            },
            FileContext {
                path: PathBuf::from("main.py"),
                relative_path: "main.py".to_string(),
                content: Arc::from(""),
            },
        ];

        let matched = filter_files_by_patterns(&files, &["**/*.ts", "**/*.tsx"]);
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn test_filter_files_empty_patterns() {
        let files = vec![FileContext {
            path: PathBuf::from("file.ts"),
            relative_path: "file.ts".to_string(),
            content: Arc::from(""),
        }];

        let matched = filter_files_by_patterns(&files, &[]);
        assert!(matched.is_empty());
    }
}
