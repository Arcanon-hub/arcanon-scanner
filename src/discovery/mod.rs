use anyhow::Result;
use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};

/// Built-in excludes that cannot be overridden by .gitignore allow patterns.
/// These are patterns that should always be excluded to avoid scanning generated
/// code, dependencies, build artifacts, and version control metadata.
pub const BUILT_IN_EXCLUDES: &[&str] = &[
    ".git/",
    "node_modules/",
    "__pycache__/",
    ".tox/",
    ".mypy_cache/",
    ".pytest_cache/",
    "target/",
    "dist/",
    "build/",
    "out/",
    ".next/",
    "vendor/",
];

/// Walks a repository directory and returns all files that pass all filters.
///
/// The walker applies several filters in order:
/// 1. Built-in excludes (.git/, node_modules/, etc.) - CANNOT be overridden by .gitignore
/// 2. Respects nested .gitignore files at every directory level
/// 3. User-supplied excludes from config or CLI
/// 4. Does not follow symlinks (DISC-05)
/// 5. Skips files exceeding 500KB (DISC-03)
/// 6. Skips binary files (null bytes in first 8KB) (DISC-03)
/// 7. Skips files with first line exceeding 10,000 characters (DISC-03)
///
/// # Arguments
/// * `root` - The root directory to walk
/// * `excludes` - Additional exclude patterns in gitignore syntax
///
/// # Returns
/// A vector of PathBuf for all files that passed all filters, or an error if
/// the walk failed.
pub fn walk_repo(root: &Path, excludes: &[String]) -> Result<Vec<PathBuf>> {
    // Build the OverrideBuilder with all exclusion patterns
    // Note: OverrideBuilder uses inverted gitignore semantics — "!" prefix means EXCLUDE
    let mut overrides = OverrideBuilder::new(root);

    // Add built-in excludes (hard-coded, cannot be overridden by .gitignore)
    for pattern in BUILT_IN_EXCLUDES {
        overrides.add(&format!("!{}", pattern))?;
    }

    // Add user-supplied excludes
    for pattern in excludes {
        overrides.add(&format!("!{}", pattern))?;
    }

    let overrides = overrides.build()?;

    // Configure the walker
    let walker = WalkBuilder::new(root)
        .follow_links(false) // DISC-05: never follow symlinks
        .max_filesize(Some(512_000)) // DISC-03: 500KB = 512,000 bytes
        .overrides(overrides)
        .build();

    let mut files = Vec::new();

    // Walk the directory tree
    for result in walker {
        let entry = result?;

        // Only process files (not directories)
        if entry.file_type().map_or(false, |ft| ft.is_file()) {
            // Apply content guards (binary, line-length checks)
            if let Some(path) = passes_content_guards(entry.path()) {
                files.push(path.to_owned());
            }
        }
    }

    Ok(files)
}

/// Checks if a file passes content guards: binary detection and line-length validation.
///
/// Returns Some(path) if the file passes all guards, None otherwise.
///
/// Guards applied:
/// - DISC-03: Binary guard — if any byte in the first 8KB is 0u8, the file is considered binary and skipped
/// - DISC-03: Line-length guard — if the first line exceeds 10,000 characters, the file is skipped (minification guard)
fn passes_content_guards(path: &Path) -> Option<&Path> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 8192];
    let n = std::io::Read::read(&mut file, &mut buf).ok()?;

    // Binary guard: check for null bytes in the first 8KB
    if buf[..n].contains(&0u8) {
        tracing::debug!("skipping binary file: {}", path.display());
        return None;
    }

    // Line-length guard: find the position of the first newline
    // If the first line is longer than 10,000 characters, skip it
    match buf[..n].iter().position(|&b| b == b'\n') {
        Some(pos) if pos > 10_000 => {
            tracing::debug!("skipping long-line file: {}", path.display());
            return None;
        }
        None if n == 8192 => {
            // Buffer is full with no newline — line is definitely >8KB, likely minified
            tracing::debug!("skipping long-line file: {}", path.display());
            return None;
        }
        _ => {}
    }

    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_built_in_excludes_constant() {
        assert!(!BUILT_IN_EXCLUDES.is_empty());
        assert!(BUILT_IN_EXCLUDES.contains(&".git/"));
        assert!(BUILT_IN_EXCLUDES.contains(&"node_modules/"));
    }
}
