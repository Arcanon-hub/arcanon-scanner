use anyhow::Result;
use gix::remote::Direction;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Git context information about a repository.
///
/// Contains the remote URL, repository name, branch name, and commit SHA.
/// Each field has a fallback chain to handle different CI environments and
/// missing git metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitContext {
    /// Remote URL (None if not a git repo or no remotes)
    pub repo_url: Option<String>,
    /// Repo name derived from URL basename or directory name
    pub repo_name: String,
    /// Branch name with full fallback chain
    pub branch: String,
    /// 40-char hex commit SHA with full fallback chain
    pub commit_sha: String,
}

/// Detects git context for a given root directory.
///
/// This function attempts to discover git metadata in the following order:
/// 1. Use gix to discover the repository and extract metadata
/// 2. Fall back to environment variables for branch and commit SHA
/// 3. Use a content hash fallback when no git context is available
///
/// # Arguments
/// * `root` - The root directory to analyze
///
/// # Returns
/// A GitContext struct with repo metadata, or an error if git discovery fails unexpectedly
pub fn detect_git_context(root: &Path) -> Result<GitContext> {
    // Try to discover the git repository
    let repo = gix::discover(root).ok();

    // Detect remote URL and derive repo name
    let (repo_url, repo_name) = detect_remote(repo.as_ref(), root);

    // Detect branch name with fallback chain
    let branch = detect_branch(repo.as_ref());

    // Detect commit SHA with fallback chain
    let commit_sha = detect_commit_sha(repo.as_ref(), root);

    Ok(GitContext {
        repo_url,
        repo_name,
        branch,
        commit_sha,
    })
}

/// Detects the remote URL and derives the repo name.
///
/// Returns a tuple of (repo_url, repo_name).
/// repo_url is None if the repo is not a git repo or has no remotes.
/// repo_name falls back to the directory name if no remote URL is available.
fn detect_remote(repo: Option<&gix::Repository>, root: &Path) -> (Option<String>, String) {
    let repo_url = repo
        .and_then(|r| {
            // Try to find "origin" remote first
            r.try_find_remote("origin")
                .transpose()
                .ok()
                .flatten()
                .or_else(|| {
                    // Fall back to the first available remote
                    let remote_names = r.remote_names().into_iter().next();
                    remote_names.and_then(|name| {
                        r.try_find_remote(name.as_ref()).transpose().ok().flatten()
                    })
                })
        })
        .and_then(|remote| {
            remote
                .url(Direction::Fetch)
                .cloned()
                .map(|url| format!("{url}"))
        });

    // Derive repo_name from URL or use directory name as fallback
    let repo_name = repo_url
        .as_ref()
        .and_then(|url| {
            // Extract basename from URL: split on '/', take last segment, strip .git
            url.split('/')
                .last()
                .map(|segment| segment.strip_suffix(".git").unwrap_or(segment).to_string())
        })
        .unwrap_or_else(|| {
            // Fallback: use directory basename
            root.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string()
        });

    (repo_url, repo_name)
}

/// Detects the branch name with a full fallback chain.
///
/// Fallback chain order:
/// 1. Git repository HEAD (if available and not detached)
/// 2. ARCANON_BRANCH environment variable
/// 3. GITHUB_REF_NAME environment variable (GitHub Actions)
/// 4. CI_COMMIT_BRANCH environment variable (GitLab CI)
/// 5. BRANCH_NAME environment variable (Jenkins)
/// 6. Default "detached" if no branch is found
fn detect_branch(repo: Option<&gix::Repository>) -> String {
    // Try to get branch from git
    if let Some(repo) = repo {
        if let Ok(head) = repo.head() {
            if let Some(name) = head.referent_name() {
                let branch_name = name.shorten().to_string();
                if !branch_name.is_empty() {
                    return branch_name;
                }
            }
        }
    }

    // Fallback chain for environment variables
    let env_vars = [
        "ARCANON_BRANCH",
        "GITHUB_REF_NAME",
        "CI_COMMIT_BRANCH",
        "BRANCH_NAME",
    ];

    for var in &env_vars {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return val;
            }
        }
    }

    // Final fallback
    tracing::warn!(
        "Could not detect branch name; using 'detached'. Set ARCANON_BRANCH or GITHUB_REF_NAME."
    );
    "detached".to_string()
}

/// Detects the commit SHA with a full fallback chain.
///
/// Fallback chain order:
/// 1. Git repository HEAD commit ID (40-char hex)
/// 2. ARCANON_COMMIT_SHA environment variable
/// 3. GITHUB_SHA environment variable (GitHub Actions)
/// 4. CI_COMMIT_SHA environment variable (GitLab CI)
/// 5. GIT_COMMIT environment variable (Jenkins)
/// 6. Content hash fallback (SHA-256 of file contents)
fn detect_commit_sha(repo: Option<&gix::Repository>, root: &Path) -> String {
    // Try to get commit SHA from git
    if let Some(repo) = repo {
        if let Ok(head_id) = repo.head_id() {
            let sha = head_id.to_hex().to_string();
            if !sha.is_empty() {
                return sha;
            }
        }
    }

    // Fallback chain for environment variables
    let env_vars = [
        "ARCANON_COMMIT_SHA",
        "GITHUB_SHA",
        "CI_COMMIT_SHA",
        "GIT_COMMIT",
    ];

    for var in &env_vars {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return val;
            }
        }
    }

    // Final fallback: compute content hash
    tracing::warn!("Could not detect commit SHA; using content hash fallback.");
    content_hash_fallback(root)
}

/// Computes a deterministic SHA-256 hash of the directory contents.
///
/// This function walks the directory tree, collecting relative file paths and sizes,
/// sorts them, and computes a SHA-256 hash of the concatenated "path:size\n" entries.
///
/// Modification times are intentionally excluded to ensure deterministic hashes across CI runs.
///
/// # Arguments
/// * `root` - The root directory to hash
///
/// # Returns
/// A lowercase 64-character hex string representing the SHA-256 hash
fn content_hash_fallback(root: &Path) -> String {
    let mut entries: BTreeMap<String, u64> = BTreeMap::new();

    // Walk the directory recursively and collect file paths and sizes
    if let Ok(_) = fs::read_dir(root) {
        fn walk_dir(
            dir: &Path,
            root: &Path,
            entries: &mut BTreeMap<String, u64>,
        ) -> std::io::Result<()> {
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    // Recursively walk subdirectories
                    walk_dir(&path, root, entries)?;
                } else if path.is_file() {
                    // Get the relative path and file size
                    if let Ok(rel_path) = path.strip_prefix(root) {
                        if let Ok(metadata) = fs::metadata(&path) {
                            let rel_path_str = rel_path.to_string_lossy().replace("\\", "/"); // Normalize to Unix-style paths
                            entries.insert(rel_path_str, metadata.len());
                        }
                    }
                }
            }
            Ok(())
        }

        let _ = walk_dir(root, root, &mut entries);
    }

    // Build the content string
    let mut content = String::new();
    for (path, size) in entries {
        content.push_str(&format!("{}:{}\n", path, size));
    }

    // Compute SHA-256
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_context_struct() {
        let ctx = GitContext {
            repo_url: Some("https://github.com/example/repo.git".to_string()),
            repo_name: "repo".to_string(),
            branch: "main".to_string(),
            commit_sha: "abc123def456".to_string(),
        };
        assert_eq!(ctx.branch, "main");
        assert_eq!(ctx.repo_name, "repo");
    }
}
