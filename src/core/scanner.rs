//! Orchestration pipeline for the scanner.
//! Wires all components: file discovery → variable store → git context → parallel plugin execution
//! → merge → resolve → payload assembly → upload/output/dry-run.
//!
//! Implements PIPE-05 (rayon parallel execution) and FTOL-02 (panic isolation per plugin).

use crate::core::{merger, payload, resolver};
use crate::discovery::walk_repo;
use crate::git::detect_git_context;
use crate::libres::{read_manifest_deps, LibraryResolver};
use crate::patterns::{PatternRegistry, PatternSource};
use crate::plugin::{default_plugins, ExtractionContext, FileContext, LanguagePlugin};
use crate::types::ExtractionResult;
use crate::vars::build_variable_store;
use crate::wrapper;
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
    /// User-defined pattern overrides from .arcanon.toml [[patterns]]
    pub user_pattern_overrides: Vec<crate::config::PatternOverride>,
    /// Pattern IDs to disable from .arcanon.toml [scanner.patterns] disabled
    pub disabled_patterns: Vec<String>,
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
/// 1. Loads pattern registry from remote or cache
/// 2. Records start time
/// 3. Detects git context
/// 4. Builds variable store
/// 5. Discovers files
/// 6. Filters plugins by name
/// 7. Runs plugins in parallel with panic isolation (rayon + catch_unwind)
/// 8. Runs pattern engine for each language
/// 9. Merges plugin + pattern results
/// 10. Checks for empty findings (warning)
/// 11. Applies service overrides
/// 12. Resolves intra-repo connections
/// 13. Assembles ScanPayloadV1
/// 14. Returns payload
pub async fn run(config: &ScannerConfig) -> Result<payload::ScanPayloadV1> {
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

    // Step 3b: Load pattern registry (PTRN-01, PTRN-02, PTRN-03)
    let pattern_registry = PatternRegistry::load(Some(&config.hub_url)).await;

    // Apply user overrides (D-11: override by ID) and disabled filter (D-12)
    let pattern_registry = pattern_registry
        .with_overrides(&config.user_pattern_overrides)
        .with_disabled(&config.disabled_patterns);

    let pattern_version = pattern_registry.version.clone();
    let pattern_source = match &pattern_registry.source {
        PatternSource::Remote => "remote".to_string(),
        PatternSource::Cache => "cache".to_string(),
        PatternSource::None => "none".to_string(),
    };

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

    // Build service_roots from config plugin results for monorepo scoping (MONO-01).
    // First, extract config plugins and run them to discover services.
    let config_plugins: Vec<&dyn LanguagePlugin> = plugins
        .iter()
        .filter(|p| is_config_plugin(p.name()))
        .map(|b| b.as_ref())
        .collect();

    let config_results = run_plugins_parallel(
        &config_plugins,
        &all_files,
        &vars_arc,
        &config.root,
        &HashMap::new(), // Config plugins don't need service_roots yet
    );

    let service_roots: HashMap<PathBuf, String> = config_results
        .iter()
        .flat_map(|r| r.services.iter())
        .map(|s| (config.root.join(&s.root_path), s.name.clone()))
        .collect();

    debug!(
        "Built service_roots map with {} entries",
        service_roots.len()
    );

    // Run all plugins (config + language) with service_roots available
    let all_results = run_plugins_parallel(
        &plugins.iter().map(|p| p.as_ref()).collect::<Vec<_>>(),
        &all_files,
        &vars_arc,
        &config.root,
        &service_roots,
    );

    info!("Plugin execution complete: {} results", all_results.len());

    // Library resolution — one resolver per scan, shared cache (D-10)
    let mut lib_resolver = LibraryResolver::new(&config.root);

    // Step 7: Run pattern engine for each language (PTRN-05, PTRN-06)
    let language_map: &[(&str, &[&str])] = &[
        (
            "typescript",
            &["**/*.ts", "**/*.tsx", "**/*.js", "**/*.jsx"],
        ),
        ("python", &["**/*.py"]),
        ("go", &["**/*.go"]),
        ("java", &["**/*.java"]),
        ("csharp", &["**/*.cs"]),
        ("rust", &["**/*.rs"]),
        ("ruby", &["**/*.rb"]),
    ];

    let mut pattern_results: Vec<ExtractionResult> = Vec::new();
    for (language, patterns) in language_map {
        let lang_files = filter_files_by_patterns(&all_files, patterns);
        if !lang_files.is_empty() {
            // Run pattern engine on user source files
            let result = pattern_registry.apply_all(&lang_files, language, &service_roots);
            if !result.connections.is_empty() {
                debug!(
                    "Pattern engine: {} {} connections",
                    result.connections.len(),
                    language
                );
                pattern_results.push(result);
            }

            // Run library resolution on production deps from manifest (LRES-01/02/03)
            let dep_names = read_manifest_deps(&config.root, language);
            if !dep_names.is_empty() {
                let resolved_libs = lib_resolver.resolve_for_language(
                    &pattern_registry,
                    language,
                    &dep_names,
                    &service_roots,
                );

                if !resolved_libs.is_empty() {
                    // Deduplicated: emit one ConnectionInfo per (library, protocol, service) — DACC-03
                    let libres_connections =
                        build_libres_connections(&resolved_libs, &lang_files, &service_roots);

                    if !libres_connections.is_empty() {
                        debug!(
                            "Library resolution: {} connections from {} libraries ({})",
                            libres_connections.len(),
                            resolved_libs.len(),
                            language
                        );
                        pattern_results.push(crate::types::ExtractionResult {
                            connections: libres_connections,
                            ..Default::default()
                        });
                    }
                }
            }
        }
    }

    // Wrapper tracing — two-pass call graph analysis (Phase 7)
    // Build wrapper map PER LANGUAGE to avoid cross-language false positives
    {
        let lib_files_for_wrapper: Vec<(String, Vec<FileContext>)> = Vec::new();

        for (language, patterns) in language_map {
            let lang_files = filter_files_by_patterns(&all_files, patterns);
            if lang_files.is_empty() {
                continue;
            }

            // Build language-scoped wrapper map
            let wrapper_map = wrapper::build_wrapper_map(
                &lang_files,
                &lib_files_for_wrapper,
                &pattern_registry,
                language,
            );

            if !wrapper_map.is_empty() {
                debug!(
                    "Wrapper map for {}: {} entries",
                    language,
                    wrapper_map.len()
                );

                {
                    let wrapper_result =
                        wrapper::detect_wrapper_calls(&lang_files, &wrapper_map, &service_roots);
                    if !wrapper_result.connections.is_empty() {
                        debug!(
                            "Wrapper tracing: {} {} connections",
                            wrapper_result.connections.len(),
                            language
                        );
                        pattern_results.push(wrapper_result);
                    }
                }
            }
        }
    }

    // Deduplicate pattern + wrapper connections: prefer pattern-engine over wrapper_trace
    // when both detect the same (source_file_base, protocol) pair (WRAP-11).
    // source_file for wrapper has format "path:line" — strip line suffix for comparison.
    {
        use std::collections::HashSet;
        // First pass: collect all (source_file_base, protocol) keys from non-wrapper results
        let mut pattern_keys: HashSet<(String, String)> = HashSet::new();
        for result in &pattern_results {
            for conn in &result.connections {
                if !conn.extraction_method.starts_with("wrapper_trace:") {
                    let base = conn.source_file.split(':').next().unwrap_or(&conn.source_file).to_string();
                    pattern_keys.insert((base, conn.protocol.clone()));
                }
            }
        }

        // Second pass: filter wrapper connections that duplicate a pattern-engine connection
        if !pattern_keys.is_empty() {
            for result in &mut pattern_results {
                result.connections.retain(|conn| {
                    if conn.extraction_method.starts_with("wrapper_trace:") {
                        let base = conn.source_file.split(':').next().unwrap_or(&conn.source_file).to_string();
                        let key = (base, conn.protocol.clone());
                        // Retain wrapper connection only if there is NO pattern-engine match
                        !pattern_keys.contains(&key)
                    } else {
                        true
                    }
                });
            }
        }
    }

    // Merge plugin results + pattern results together (PTRN-06)
    let combined_results: Vec<ExtractionResult> =
        all_results.into_iter().chain(pattern_results).collect();
    let results = combined_results;

    // Step 8: Merge all results
    let mut merged = merger::merge(results);
    debug!("Merged into {} services", merged.services.len());

    // Step 9: Infer service from repo if connections exist but no services detected
    merger::infer_service_if_needed(&mut merged, &git_context.repo_name);

    // Step 10: Apply service overrides
    merger::apply_service_overrides(&mut merged, &config.service_overrides);

    // Step 11: Resolve intra-repo connections
    let merged = resolver::resolve(merged);
    debug!("Resolved connections");

    // Step 12: Record end time and assemble payload
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
        pattern_version,
        pattern_source,
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
    plugins: &[&dyn LanguagePlugin],
    all_files: &[FileContext],
    vars: &Arc<crate::vars::VariableStore>,
    root: &Path,
    service_roots: &HashMap<PathBuf, String>,
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
                service_roots: service_roots.clone(),
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

/// Check if a plugin is a config plugin (runs always and provides ServiceInfo).
fn is_config_plugin(name: &str) -> bool {
    matches!(
        name,
        "openapi"
            | "proto"
            | "graphql"
            | "asyncapi"
            | "compose"
            | "kubernetes"
            | "dockerfile"
            | "env"
    )
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

/// Check if a source line is an import statement for the given library name.
/// Handles Python (import/from), Node (require/import), Ruby (require), Rust (use/extern crate), Go (import), Java/C# (using/import).
fn line_contains_import(line: &str, lib_name: &str) -> bool {
    // Normalize lib name: replace _ with - and vice versa for matching
    let normalized = lib_name.replace('-', "_");
    let hyphenated = lib_name.replace('_', "-");

    let candidates = [lib_name, normalized.as_str(), hyphenated.as_str()];

    for candidate in candidates {
        // Python: import lib_name / from lib_name import ...
        if line.starts_with("import ") && line.contains(candidate) {
            return true;
        }
        if line.starts_with("from ") && line.contains(candidate) {
            return true;
        }
        // Node: require('lib') / import ... from 'lib'
        if line.contains("require(") && line.contains(candidate) {
            return true;
        }
        if line.contains("from '") && line.contains(candidate) {
            return true;
        }
        if line.contains("from \"") && line.contains(candidate) {
            return true;
        }
        // Ruby: require 'lib'
        if line.starts_with("require '") && line.contains(candidate) {
            return true;
        }
        if line.starts_with("require \"") && line.contains(candidate) {
            return true;
        }
        // Rust: use lib_name:: / extern crate lib_name
        if line.starts_with("use ") && line.contains(candidate) {
            return true;
        }
        if line.starts_with("extern crate ") && line.contains(candidate) {
            return true;
        }
        // Go: import block handled as content contains check
        if line.contains('"')
            && line.contains(candidate)
            && (line.trim_start().starts_with('"') || line.contains("import "))
        {
            return true;
        }
    }
    false
}

/// Build deduplicated library resolution connections.
///
/// Emits exactly one `ConnectionInfo` per (library, protocol, source_service) triple.
/// Multiple import lines for the same library in the same service (or file) are collapsed
/// to a single connection. This eliminates the per-import-line amplification in DACC-03.
pub fn build_libres_connections(
    resolved_libs: &[crate::libres::ResolvedLibrary],
    lang_files: &[FileContext],
    service_roots: &HashMap<PathBuf, String>,
) -> Vec<crate::types::ConnectionInfo> {
    use std::collections::HashSet;
    let mut libres_connections: Vec<crate::types::ConnectionInfo> = Vec::new();
    let mut seen: HashSet<(String, String, String)> = HashSet::new();

    for resolved in resolved_libs {
        for protocol in &resolved.protocols {
            for file in lang_files {
                // Find the first import line for this library in this file
                let first_import = file
                    .content
                    .lines()
                    .find(|line| line_contains_import(line.trim(), &resolved.lib_name));

                if let Some(evidence_line) = first_import {
                    let source_service = crate::plugin::scope_to_service(&file.path, service_roots)
                        .unwrap_or("")
                        .to_string();
                    let key = (
                        resolved.lib_name.clone(),
                        protocol.clone(),
                        source_service.clone(),
                    );
                    if seen.insert(key) {
                        libres_connections.push(crate::types::ConnectionInfo {
                            source_service,
                            target_name: String::new(),
                            protocol: protocol.clone(),
                            method: None,
                            path: None,
                            source_file: file.relative_path.clone(),
                            confidence: crate::types::Confidence::Medium,
                            extraction_method: format!(
                                "library_resolution:{}→{}",
                                resolved.lib_name, protocol
                            ),
                            evidence: Some(evidence_line.trim().to_string()),
                        });
                    }
                }
            }
        }
    }
    libres_connections
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

    // --- DACC-03 deduplication tests ---

    fn make_resolved(lib_name: &str, protocols: &[&str]) -> crate::libres::ResolvedLibrary {
        crate::libres::ResolvedLibrary {
            lib_name: lib_name.to_string(),
            protocols: protocols.iter().map(|s| s.to_string()).collect(),
            source_file_hint: String::new(),
        }
    }

    fn make_file(path: &str, content: &str) -> FileContext {
        FileContext {
            path: PathBuf::from(path),
            relative_path: path.to_string(),
            content: Arc::from(content),
        }
    }

    /// Test 1: a service with 10 `import asyncua` lines produces 1 connection, not 10
    #[test]
    fn test_dacc03_ten_import_lines_produce_one_connection() {
        let resolved = vec![make_resolved("asyncua", &["opcua"])];
        // Build a file with 10 identical import lines
        let content = "import asyncua\n".repeat(10);
        let files = vec![make_file("svc/client.py", &content)];
        let service_roots = HashMap::new();

        let conns = build_libres_connections(&resolved, &files, &service_roots);
        assert_eq!(
            conns.len(),
            1,
            "10 import lines in one file should produce exactly 1 connection"
        );
        assert_eq!(conns[0].protocol, "opcua");
    }

    /// Test 2: two separate services importing the same library produce 2 connections (one per service)
    #[test]
    fn test_dacc03_two_services_produce_two_connections() {
        let resolved = vec![make_resolved("asyncua", &["opcua"])];
        let files = vec![
            make_file("svc-a/client.py", "import asyncua\nimport asyncua\n"),
            make_file("svc-b/worker.py", "import asyncua\n"),
        ];
        // Set up service_roots so each directory maps to a separate service
        let mut service_roots = HashMap::new();
        service_roots.insert(
            PathBuf::from("svc-a"),
            "service-a".to_string(),
        );
        service_roots.insert(
            PathBuf::from("svc-b"),
            "service-b".to_string(),
        );

        let conns = build_libres_connections(&resolved, &files, &service_roots);
        assert_eq!(
            conns.len(),
            2,
            "two services importing the same library should produce 2 connections"
        );
        let sources: Vec<&str> = conns.iter().map(|c| c.source_service.as_str()).collect();
        assert!(sources.contains(&"service-a"), "should include service-a");
        assert!(sources.contains(&"service-b"), "should include service-b");
    }

    /// Test 3: a library with two protocols produces 2 connections per service
    #[test]
    fn test_dacc03_two_protocols_produce_two_connections() {
        let resolved = vec![make_resolved("redis", &["redis", "redis+tls"])];
        let files = vec![make_file("app/main.py", "import redis\nimport redis\n")];
        let service_roots = HashMap::new();

        let conns = build_libres_connections(&resolved, &files, &service_roots);
        assert_eq!(
            conns.len(),
            2,
            "one library with two protocols should produce 2 connections per service"
        );
        let protocols: Vec<&str> = conns.iter().map(|c| c.protocol.as_str()).collect();
        assert!(protocols.contains(&"redis"), "should have redis protocol");
        assert!(
            protocols.contains(&"redis+tls"),
            "should have redis+tls protocol"
        );
    }
}
