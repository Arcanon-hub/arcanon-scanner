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
}
