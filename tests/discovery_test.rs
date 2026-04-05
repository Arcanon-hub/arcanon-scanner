use arcanon::discovery::walk_repo;
#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn test_builtin_excludes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
    std::fs::write(dir.path().join("node_modules/pkg.js"), b"module").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.ts"), b"export {}").unwrap();

    let files = walk_repo(dir.path(), &[]).unwrap();
    let names: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(names.contains(&"main.ts"), "should include src/main.ts");
    assert!(
        !names.contains(&"pkg.js"),
        "should exclude node_modules/pkg.js"
    );
}

#[test]
fn test_all_builtin_excludes() {
    let dir = tempfile::tempdir().unwrap();
    let excludes = [
        "target",
        "__pycache__",
        ".git",
        "vendor",
        "dist",
        "build",
        "out",
        ".next",
        ".tox",
        ".mypy_cache",
        ".pytest_cache",
    ];
    for exc in &excludes {
        std::fs::create_dir_all(dir.path().join(exc)).unwrap();
        std::fs::write(dir.path().join(format!("{}/file.txt", exc)), b"data").unwrap();
    }
    std::fs::write(dir.path().join("root.txt"), b"data").unwrap();

    let files = walk_repo(dir.path(), &[]).unwrap();
    let names: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    assert!(
        names.contains(&"root.txt".to_string()),
        "root.txt should be included"
    );
    assert_eq!(
        names.iter().filter(|n| *n == "file.txt").count(),
        0,
        "no file.txt from excluded dirs should appear"
    );
}

#[test]
fn test_user_excludes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.log"), b"log data").unwrap();
    std::fs::write(dir.path().join("app.rs"), b"fn main() {}").unwrap();

    let excludes = vec!["*.log".to_string()];
    let files = walk_repo(dir.path(), &excludes).unwrap();
    let names: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(names.contains(&"app.rs"));
    assert!(!names.contains(&"app.log"));
}

#[test]
fn test_binary_guard() {
    let dir = tempfile::tempdir().unwrap();
    let mut content = vec![b'a'; 200];
    content[100] = 0u8; // null byte — binary signal
    std::fs::write(dir.path().join("binary.bin"), &content).unwrap();
    std::fs::write(dir.path().join("text.txt"), b"hello world").unwrap();

    let files = walk_repo(dir.path(), &[]).unwrap();
    let names: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(names.contains(&"text.txt"));
    assert!(
        !names.contains(&"binary.bin"),
        "binary file should be excluded"
    );
}

#[test]
fn test_line_length_guard() {
    let dir = tempfile::tempdir().unwrap();
    // First line is 10,001 'a' chars — exceeds 10,000 limit
    let long_line = "a".repeat(10_001);
    std::fs::write(dir.path().join("minified.js"), long_line.as_bytes()).unwrap();
    std::fs::write(dir.path().join("normal.js"), b"const x = 1;").unwrap();

    let files = walk_repo(dir.path(), &[]).unwrap();
    let names: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(names.contains(&"normal.js"));
    assert!(
        !names.contains(&"minified.js"),
        "long-line file should be excluded"
    );
}

#[test]
#[cfg(unix)]
fn test_no_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("real.txt"), b"real content").unwrap();
    symlink(dir.path().join("real.txt"), dir.path().join("link.txt")).unwrap();

    let files = walk_repo(dir.path(), &[]).unwrap();
    let names: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    // real.txt may or may not be included, but link.txt (the symlink) must not be followed/returned
    assert!(
        !names.contains(&"link.txt"),
        "symlink should not be followed"
    );
}

#[test]
fn test_nested_gitignore() {
    let dir = tempfile::tempdir().unwrap();

    // Initialize git repo (ignore crate respects .gitignore in git repos)
    std::process::Command::new("git")
        .args(&["init"])
        .current_dir(dir.path())
        .output()
        .ok();

    std::fs::create_dir_all(dir.path().join("subdir")).unwrap();
    std::fs::write(dir.path().join("subdir/.gitignore"), b"*.tmp\n").unwrap();
    std::fs::write(dir.path().join("subdir/cache.tmp"), b"temp data").unwrap();
    std::fs::write(dir.path().join("subdir/app.rs"), b"fn main() {}").unwrap();

    let files = walk_repo(dir.path(), &[]).unwrap();
    let names: Vec<_> = files
        .iter()
        .map(|p| p.file_name().unwrap().to_str().unwrap())
        .collect();
    assert!(names.contains(&"app.rs"), "app.rs should be included");
    assert!(
        !names.contains(&"cache.tmp"),
        "*.tmp ignored by nested .gitignore should be excluded"
    );
}
