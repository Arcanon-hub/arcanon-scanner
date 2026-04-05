//! Library resolution module — discovers and analyzes installed package environments
//! to identify which known connection protocols custom/internal libraries wrap.
//!
//! # Strategy
//!
//! For each dependency in a project manifest (pyproject.toml, package.json, Cargo.toml, etc.):
//! 1. **Environment discovery**: Find installed source (Python venv, node_modules, Ruby vendor)
//! 2. **Source scan**: Walk library source files and run pattern engine
//! 3. **Lock file fallback**: For Go/Rust/Java/C#, parse lock files (Cargo.lock, go.mod, etc.)
//! 4. **Blocklist filtering**: Skip known non-connection libraries (numpy, react, pytest, etc.)
//! 5. **Caching**: Store protocols per library to avoid re-scanning
//!
//! # Constraints
//!
//! - **Synchronous**: No tokio. All file I/O via std::fs. Can be called from async context.
//! - **No rayon**: Library resolution runs on rayon's thread pool but uses sync I/O.
//! - **Graceful failure**: Missing environments log at info level, return empty results.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Libraries that are frameworks, tools, or data processing libraries — not connection wrappers.
/// Prevents false positives from scanning numpy, pytest, react, webpack, etc.
const BLOCKLIST: &[&str] = &[
    "numpy",
    "pandas",
    "scipy",
    "matplotlib",
    "pillow",
    "django",
    "flask",
    "fastapi",
    "react",
    "vue",
    "angular",
    "svelte",
    "express",
    "nestjs",
    "next",
    "nuxt",
    "gatsby",
    "pytest",
    "unittest",
    "eslint",
    "prettier",
    "webpack",
    "vite",
    "rollup",
    "babel",
    "jest",
    "mocha",
    "vitest",
    "setuptools",
    "pip",
    "poetry",
    "cargo",
    "rustc",
    "go",
    "javac",
];

/// A library resolved to contain connections with known protocols.
#[derive(Debug, Clone)]
pub struct ResolvedLibrary {
    /// Library name from manifest.
    pub lib_name: String,
    /// Detected protocols: ["rest", "grpc", "postgresql", "redis", etc.].
    pub protocols: Vec<String>,
    /// Hint about source: "library_resolution:libname→protocol" style.
    pub source_file_hint: String,
}

/// Known connection libraries and their associated protocols.
/// Maps dependency name fragment to protocol string.
const KNOWN_CONNECTION_LIBS: &[(&str, &str)] = &[
    ("reqwest", "rest"),
    ("tonic", "grpc"),
    ("tokio-postgres", "postgresql"),
    ("sqlx", "postgresql"),
    ("redis", "redis"),
    ("rdkafka", "kafka"),
    ("rumqttc", "mqtt"),
    ("lapin", "amqp"),
    ("httpx", "rest"),
    ("aiohttp", "rest"),
    ("requests", "rest"),
    ("psycopg", "postgresql"),
    ("pymongo", "mongodb"),
    ("motor", "mongodb"),
    ("boto3", "sqs"),
    ("aiokafka", "kafka"),
    ("pika", "amqp"),
    ("axios", "rest"),
    ("node-fetch", "rest"),
    ("got", "rest"),
    ("ky", "rest"),
    ("pg", "postgresql"),
    ("mongoose", "mongodb"),
    ("ioredis", "redis"),
    ("mysql2", "mysql"),
    ("kafkajs", "kafka"),
    ("amqplib", "amqp"),
    ("mqtt", "mqtt"),
    ("grpc", "grpc"),
    ("google.golang.org/grpc", "grpc"),
    ("net/http", "rest"),
    ("RestTemplate", "rest"),
    ("WebClient", "rest"),
    ("OkHttp", "rest"),
    ("Faraday", "rest"),
    ("typhoeus", "rest"),
];

/// Extract protocols from a dependency name by checking known connection libraries.
fn dep_to_protocols(dep_name: &str) -> Vec<String> {
    let lower = dep_name.to_lowercase();
    let mut protocols = Vec::new();
    for (fragment, protocol) in KNOWN_CONNECTION_LIBS {
        if lower.contains(&fragment.to_lowercase()) {
            if !protocols.contains(&protocol.to_string()) {
                protocols.push(protocol.to_string());
            }
        }
    }
    protocols
}

/// Infer protocols from a list of dependency names.
/// Returns deduplicated, sorted list of protocol strings.
pub fn infer_protocols_from_deps(dep_names: &[String]) -> Vec<String> {
    let mut protocols = std::collections::HashSet::new();
    for dep in dep_names {
        for protocol in dep_to_protocols(dep) {
            protocols.insert(protocol);
        }
    }
    let mut result: Vec<String> = protocols.into_iter().collect();
    result.sort();
    result
}

/// Discovers and resolves custom/internal libraries to the protocols they wrap.
pub struct LibraryResolver {
    /// Repository root path.
    root: PathBuf,
    /// Cache of library → protocols. Empty vec = confirmed not a connection lib.
    cache: HashMap<String, Vec<String>>,
}

impl LibraryResolver {
    /// Create a new LibraryResolver with the given repo root.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            cache: HashMap::new(),
        }
    }

    /// Check if a library name is in the blocklist (known non-connection library).
    fn is_blocklisted(name: &str) -> bool {
        let lower = name.to_lowercase();
        BLOCKLIST
            .iter()
            .any(|&entry| lower.starts_with(entry) || lower == entry)
    }

    /// Discover Python environment: venv/, .venv/, env/ relative to root,
    /// or VIRTUAL_ENV env var. Returns path to site-packages directory.
    fn discover_python_env(&self) -> Option<PathBuf> {
        // Check standard locations relative to root
        for dir_name in &["venv", ".venv", "env"] {
            let potential = self.root.join(dir_name).join("lib");
            if let Ok(entries) = std::fs::read_dir(&potential) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_dir() {
                            if let Some(name) = entry.file_name().to_str() {
                                if name.starts_with("python") {
                                    let site_packages = entry.path().join("site-packages");
                                    if site_packages.exists() {
                                        return Some(site_packages);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check VIRTUAL_ENV env var
        if let Ok(venv_path) = std::env::var("VIRTUAL_ENV") {
            let site_packages = PathBuf::from(&venv_path)
                .join("lib")
                .join("python3.12") // Generic fallback version
                .join("site-packages");
            if site_packages.exists() {
                return Some(site_packages);
            }

            // Try a more generic approach for VIRTUAL_ENV
            let lib_dir = PathBuf::from(&venv_path).join("lib");
            if let Ok(entries) = std::fs::read_dir(&lib_dir) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_dir() {
                            if let Some(name) = entry.file_name().to_str() {
                                if name.starts_with("python") {
                                    let site_packages = entry.path().join("site-packages");
                                    if site_packages.exists() {
                                        return Some(site_packages);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Discover Node environment: check root/node_modules/. Returns its path if it exists.
    fn discover_node_env(&self) -> Option<PathBuf> {
        let node_modules = self.root.join("node_modules");
        if node_modules.is_dir() {
            Some(node_modules)
        } else {
            None
        }
    }

    /// Discover Ruby environment: check root/vendor/bundle/ruby/*/gems/. Returns the gems path if it exists.
    fn discover_ruby_env(&self) -> Option<PathBuf> {
        let vendor_bundle = self.root.join("vendor").join("bundle").join("ruby");
        if vendor_bundle.exists() {
            if let Ok(entries) = std::fs::read_dir(&vendor_bundle) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        if meta.is_dir() {
                            let gems = entry.path().join("gems");
                            if gems.exists() {
                                return Some(gems);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Parse Cargo.lock (TOML format) to get direct dependencies per package.
    /// Returns map: package_name → [dep names].
    pub fn parse_cargo_lock(&self) -> HashMap<String, Vec<String>> {
        let lock_path = self.root.join("Cargo.lock");
        match std::fs::read_to_string(&lock_path) {
            Ok(content) => {
                match content.parse::<toml::Value>() {
                    Ok(toml::Value::Table(root)) => {
                        let mut result = HashMap::new();
                        if let Some(toml::Value::Array(packages)) = root.get("package") {
                            for pkg in packages {
                                if let toml::Value::Table(pkg_table) = pkg {
                                    if let Some(toml::Value::String(name)) = pkg_table.get("name") {
                                        let mut deps = Vec::new();
                                        if let Some(toml::Value::Array(dep_array)) =
                                            pkg_table.get("dependencies")
                                        {
                                            for dep in dep_array {
                                                if let toml::Value::String(dep_str) = dep {
                                                    // Strip version qualifier: "dep 1.0.0" → "dep"
                                                    let dep_name = dep_str
                                                        .split_whitespace()
                                                        .next()
                                                        .unwrap_or("")
                                                        .to_string();
                                                    if !dep_name.is_empty() {
                                                        deps.push(dep_name);
                                                    }
                                                }
                                            }
                                        }
                                        result.insert(name.clone(), deps);
                                    }
                                }
                            }
                        }
                        result
                    }
                    _ => {
                        tracing::debug!("Cargo.lock not valid TOML or empty");
                        HashMap::new()
                    }
                }
            }
            Err(_) => {
                tracing::debug!("Cargo.lock not found or not readable");
                HashMap::new()
            }
        }
    }

    /// Parse go.mod to extract direct dependency module paths.
    /// Returns list of module paths (e.g., "github.com/user/repo").
    pub fn parse_go_mod(&self) -> Vec<String> {
        let go_mod_path = self.root.join("go.mod");
        match std::fs::read_to_string(&go_mod_path) {
            Ok(content) => {
                let mut deps = Vec::new();
                let mut in_require_block = false;

                for line in content.lines() {
                    let trimmed = line.trim();

                    // Skip comments
                    if trimmed.starts_with("//") {
                        continue;
                    }

                    if trimmed == "require (" {
                        in_require_block = true;
                        continue;
                    }

                    if in_require_block && trimmed == ")" {
                        in_require_block = false;
                        continue;
                    }

                    if in_require_block {
                        // Parse "MODULE VERSION" line
                        let parts: Vec<&str> = trimmed.split_whitespace().collect();
                        if let Some(module) = parts.first() {
                            if !module.is_empty() {
                                deps.push(module.to_string());
                            }
                        }
                    } else if trimmed.starts_with("require ") {
                        // Single-line "require MODULE VERSION"
                        let parts: Vec<&str> = trimmed.split_whitespace().collect();
                        if parts.len() >= 2 {
                            deps.push(parts[1].to_string());
                        }
                    }
                }

                deps
            }
            Err(_) => {
                tracing::debug!("go.mod not found or not readable");
                Vec::new()
            }
        }
    }

    /// Parse Gemfile.lock to get direct dependencies per gem.
    /// Returns map: gem_name → [dep names].
    pub fn parse_gemfile_lock(&self) -> HashMap<String, Vec<String>> {
        let gemfile_lock_path = self.root.join("Gemfile.lock");
        match std::fs::read_to_string(&gemfile_lock_path) {
            Ok(content) => {
                let mut result = HashMap::new();
                let mut current_gem: Option<String> = None;
                let mut in_gem_section = false;

                for line in content.lines() {
                    // Detect GEM section start
                    if line.trim() == "GEM" {
                        in_gem_section = true;
                        continue;
                    }

                    if !in_gem_section {
                        continue;
                    }

                    // Gem entries are 4-space indented: "    gemname (1.2.3)"
                    if line.starts_with("    ") && !line.starts_with("      ") {
                        // Parse gem name
                        let trimmed = line.trim();
                        if let Some(paren_pos) = trimmed.find('(') {
                            let gem_name = trimmed[..paren_pos].trim().to_string();
                            current_gem = Some(gem_name.clone());
                            result.insert(gem_name, Vec::new());
                        }
                    } else if line.starts_with("      ") && current_gem.is_some() {
                        // Dependency entry: 6-space indented: "      depname (>= 1.0)"
                        let trimmed = line.trim();
                        if let Some(paren_pos) = trimmed.find('(') {
                            let dep_name = trimmed[..paren_pos].trim().to_string();
                            if let Some(gem) = &current_gem {
                                if let Some(deps) = result.get_mut(gem) {
                                    deps.push(dep_name);
                                }
                            }
                        }
                    }
                }

                result
            }
            Err(_) => {
                tracing::debug!("Gemfile.lock not found or not readable");
                HashMap::new()
            }
        }
    }

    /// Parse pom.xml to extract dependency identifiers.
    /// Returns list of "groupId:artifactId" strings.
    pub fn parse_pom_xml(&self) -> Vec<String> {
        let pom_path = self.root.join("pom.xml");
        match std::fs::read_to_string(&pom_path) {
            Ok(content) => {
                let mut deps = Vec::new();
                let mut in_dependency = false;
                let mut group_id = String::new();
                let mut artifact_id = String::new();
                let mut scope = String::new();

                for line in content.lines() {
                    let trimmed = line.trim();

                    if trimmed.contains("<dependency>") {
                        in_dependency = true;
                        group_id.clear();
                        artifact_id.clear();
                        scope.clear();
                        continue;
                    }

                    if trimmed.contains("</dependency>") {
                        in_dependency = false;
                        // Add to deps if scope is not test/provided
                        if !scope.contains("test")
                            && !scope.contains("provided")
                            && !group_id.is_empty()
                            && !artifact_id.is_empty()
                        {
                            deps.push(format!("{}:{}", group_id, artifact_id));
                        }
                        continue;
                    }

                    if in_dependency {
                        if trimmed.starts_with("<groupId>") && trimmed.ends_with("</groupId>") {
                            group_id = trimmed
                                .strip_prefix("<groupId>")
                                .and_then(|s| s.strip_suffix("</groupId>"))
                                .unwrap_or("")
                                .to_string();
                        }
                        if trimmed.starts_with("<artifactId>") && trimmed.ends_with("</artifactId>")
                        {
                            artifact_id = trimmed
                                .strip_prefix("<artifactId>")
                                .and_then(|s| s.strip_suffix("</artifactId>"))
                                .unwrap_or("")
                                .to_string();
                        }
                        if trimmed.starts_with("<scope>") && trimmed.ends_with("</scope>") {
                            scope = trimmed
                                .strip_prefix("<scope>")
                                .and_then(|s| s.strip_suffix("</scope>"))
                                .unwrap_or("")
                                .to_string();
                        }
                    }
                }

                deps
            }
            Err(_) => {
                tracing::debug!("pom.xml not found or not readable");
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_blocklisted_numpy() {
        assert!(LibraryResolver::is_blocklisted("numpy"));
    }

    #[test]
    fn test_is_blocklisted_pytest() {
        assert!(LibraryResolver::is_blocklisted("pytest"));
    }

    #[test]
    fn test_is_blocklisted_react() {
        assert!(LibraryResolver::is_blocklisted("react"));
    }

    #[test]
    fn test_is_blocklisted_case_insensitive() {
        assert!(LibraryResolver::is_blocklisted("NumPy"));
        assert!(LibraryResolver::is_blocklisted("PyTest"));
    }

    #[test]
    fn test_is_blocklisted_not_blocklisted() {
        assert!(!LibraryResolver::is_blocklisted("edgeworks-sdk"));
    }

    #[test]
    fn test_is_blocklisted_prefix() {
        // Should match startswith behavior
        assert!(LibraryResolver::is_blocklisted("pytest-xdist"));
    }

    #[test]
    fn test_new_initializes_empty_cache() {
        let root = PathBuf::from("/tmp");
        let resolver = LibraryResolver::new(&root);
        assert_eq!(resolver.cache.len(), 0);
    }

    #[test]
    fn test_discover_python_env_none_when_missing() {
        let resolver = LibraryResolver::new(Path::new("/nonexistent"));
        assert!(resolver.discover_python_env().is_none());
    }

    #[test]
    fn test_discover_node_env_none_when_missing() {
        let resolver = LibraryResolver::new(Path::new("/nonexistent"));
        assert!(resolver.discover_node_env().is_none());
    }

    #[test]
    fn test_discover_ruby_env_none_when_missing() {
        let resolver = LibraryResolver::new(Path::new("/nonexistent"));
        assert!(resolver.discover_ruby_env().is_none());
    }

    #[test]
    fn test_dep_to_protocols_reqwest() {
        let protocols = dep_to_protocols("reqwest");
        assert!(protocols.contains(&"rest".to_string()));
    }

    #[test]
    fn test_dep_to_protocols_tonic() {
        let protocols = dep_to_protocols("tonic");
        assert!(protocols.contains(&"grpc".to_string()));
    }

    #[test]
    fn test_dep_to_protocols_unknown() {
        let protocols = dep_to_protocols("unknown-lib");
        assert!(protocols.is_empty());
    }

    #[test]
    fn test_infer_protocols_from_deps_single() {
        let deps = vec!["reqwest".to_string()];
        let protocols = infer_protocols_from_deps(&deps);
        assert!(protocols.contains(&"rest".to_string()));
    }

    #[test]
    fn test_infer_protocols_from_deps_multiple() {
        let deps = vec!["reqwest".to_string(), "serde".to_string()];
        let protocols = infer_protocols_from_deps(&deps);
        assert!(protocols.contains(&"rest".to_string()));
        assert!(!protocols.contains(&"unknown".to_string()));
    }

    #[test]
    fn test_infer_protocols_deduplicates() {
        let deps = vec!["reqwest".to_string(), "httpx".to_string()];
        let protocols = infer_protocols_from_deps(&deps);
        let count = protocols.iter().filter(|p| *p == "rest").count();
        assert_eq!(count, 1); // Should only appear once
    }

    #[test]
    fn test_parse_cargo_lock_empty_on_missing() {
        let resolver = LibraryResolver::new(Path::new("/nonexistent"));
        let result = resolver.parse_cargo_lock();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_go_mod_empty_on_missing() {
        let resolver = LibraryResolver::new(Path::new("/nonexistent"));
        let result = resolver.parse_go_mod();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_gemfile_lock_empty_on_missing() {
        let resolver = LibraryResolver::new(Path::new("/nonexistent"));
        let result = resolver.parse_gemfile_lock();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_pom_xml_empty_on_missing() {
        let resolver = LibraryResolver::new(Path::new("/nonexistent"));
        let result = resolver.parse_pom_xml();
        assert!(result.is_empty());
    }
}
