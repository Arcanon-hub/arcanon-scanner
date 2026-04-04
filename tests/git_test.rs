use arcanon_scanner::git::detect_git_context;
use serial_test::serial;

#[test]
#[serial]
fn test_non_git_dir() {
    let dir = tempfile::tempdir().unwrap();
    // Ensure no CI env vars interfere
    std::env::remove_var("ARCANON_BRANCH");
    std::env::remove_var("GITHUB_REF_NAME");
    std::env::remove_var("CI_COMMIT_BRANCH");
    std::env::remove_var("BRANCH_NAME");
    std::env::remove_var("ARCANON_COMMIT_SHA");
    std::env::remove_var("GITHUB_SHA");
    std::env::remove_var("CI_COMMIT_SHA");
    std::env::remove_var("GIT_COMMIT");

    let ctx = detect_git_context(dir.path()).unwrap();
    assert!(
        ctx.repo_url.is_none(),
        "non-git dir should have no repo_url"
    );
    assert_eq!(
        ctx.branch, "detached",
        "non-git dir with no CI env should use 'detached'"
    );
    // repo_name should be the directory basename (temp dirs have a name)
    assert!(
        !ctx.repo_name.is_empty(),
        "repo_name should fall back to dir name"
    );
    // commit_sha should be a non-empty deterministic hash
    assert!(
        !ctx.commit_sha.is_empty(),
        "commit_sha should be content hash fallback"
    );
    assert_eq!(ctx.commit_sha.len(), 64, "SHA-256 hex should be 64 chars");
}

#[test]
#[serial]
fn test_repo_name_derivation() {
    // Test the URL parsing logic for repo_name
    // We test this by checking that detect_git_context on a dir with no git
    // uses the dir name as fallback
    let dir = tempfile::TempDir::new_in("/tmp").unwrap();
    // The dir has a system-assigned name — just verify it's non-empty
    let ctx = detect_git_context(dir.path()).unwrap();
    assert!(!ctx.repo_name.is_empty());
}

#[test]
#[serial]
fn test_branch_ci_env_fallback() {
    let dir = tempfile::tempdir().unwrap();
    // Clear gix-detectable context and set CI env var
    std::env::remove_var("ARCANON_BRANCH");
    std::env::set_var("GITHUB_REF_NAME", "feature/test-branch");
    std::env::remove_var("CI_COMMIT_BRANCH");
    std::env::remove_var("BRANCH_NAME");

    let ctx = detect_git_context(dir.path()).unwrap();
    // Non-git dir won't have a branch from gix, so GITHUB_REF_NAME should be used
    assert_eq!(
        ctx.branch, "feature/test-branch",
        "should use GITHUB_REF_NAME when gix cannot detect branch"
    );

    // Cleanup
    std::env::remove_var("GITHUB_REF_NAME");
}

#[test]
#[serial]
fn test_arcanon_branch_overrides_github_ref() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("ARCANON_BRANCH", "arcanon-override");
    std::env::set_var("GITHUB_REF_NAME", "github-branch");

    let ctx = detect_git_context(dir.path()).unwrap();
    assert_eq!(
        ctx.branch, "arcanon-override",
        "ARCANON_BRANCH should have higher priority than GITHUB_REF_NAME"
    );

    std::env::remove_var("ARCANON_BRANCH");
    std::env::remove_var("GITHUB_REF_NAME");
}

#[test]
#[serial]
fn test_content_hash_is_deterministic() {
    // Two dirs with identical content should produce the same hash
    let dir1 = tempfile::tempdir().unwrap();
    let dir2 = tempfile::tempdir().unwrap();

    for dir in &[&dir1, &dir2] {
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), b"fn main() {}").unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), b"[package]\nname = \"test\"").unwrap();
    }

    std::env::remove_var("ARCANON_BRANCH");
    std::env::remove_var("GITHUB_REF_NAME");
    std::env::remove_var("CI_COMMIT_BRANCH");
    std::env::remove_var("BRANCH_NAME");
    std::env::remove_var("ARCANON_COMMIT_SHA");
    std::env::remove_var("GITHUB_SHA");
    std::env::remove_var("CI_COMMIT_SHA");
    std::env::remove_var("GIT_COMMIT");

    let ctx1 = detect_git_context(dir1.path()).unwrap();
    let ctx2 = detect_git_context(dir2.path()).unwrap();

    // Content is identical so hashes must match
    assert_eq!(
        ctx1.commit_sha, ctx2.commit_sha,
        "same content in different dirs should produce identical content hash"
    );
}

#[test]
#[serial]
fn test_content_hash_is_64_hex_chars() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file.txt"), b"some content").unwrap();

    std::env::remove_var("ARCANON_COMMIT_SHA");
    std::env::remove_var("GITHUB_SHA");
    std::env::remove_var("CI_COMMIT_SHA");
    std::env::remove_var("GIT_COMMIT");

    let ctx = detect_git_context(dir.path()).unwrap();
    // SHA-256 produces 32 bytes = 64 hex chars
    assert_eq!(
        ctx.commit_sha.len(),
        64,
        "content hash fallback should be 64 hex chars (SHA-256)"
    );
    assert!(
        ctx.commit_sha.chars().all(|c: char| c.is_ascii_hexdigit()),
        "commit_sha should be all hex characters"
    );
}
