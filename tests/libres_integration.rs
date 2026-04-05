use arcanon::libres::{infer_protocols_from_deps, read_manifest_deps, LibraryResolver};
use arcanon::patterns::{Detection, Pattern, PatternConfidence, PatternRegistry, TargetExtraction};
use std::collections::HashMap;
use std::io::Write;
use tempfile::TempDir;

/// Helper to create a test pattern registry that detects httpx imports
fn make_httpx_registry() -> PatternRegistry {
    PatternRegistry::from_patterns(
        vec![Pattern {
            id: "py-httpx".to_string(),
            name: "httpx".to_string(),
            description: "httpx client".to_string(),
            languages: vec!["python".to_string()],
            file_patterns: vec![],
            import_gate: vec!["import httpx".to_string(), "from httpx".to_string()],
            detections: vec![Detection {
                match_str: "httpx.Client(".to_string(),
                kind: "connection".to_string(),
                protocol: "rest".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::FirstStringArg,
            }],
        }],
        "test-1.0".to_string(),
    )
}

/// Helper to write a file into a TempDir, creating parent directories as needed
fn write_file(dir: &TempDir, relative_path: &str, content: &str) {
    let path = dir.path().join(relative_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
}

/// LRES-01: Python venv discovery — finds edgeworks_sdk in venv, detects httpx wrapping
#[test]
fn test_lres01_python_venv_detection() {
    let dir = TempDir::new().unwrap();
    write_file(
        &dir,
        "venv/lib/python3.12/site-packages/edgeworks_sdk/client.py",
        "import httpx\n\ndef create_client(url):\n    return httpx.Client(url)\n",
    );

    let registry = make_httpx_registry();
    let mut resolver = LibraryResolver::new(dir.path());
    let results = resolver.resolve_for_language(
        &registry,
        "python",
        &["edgeworks_sdk".to_string()],
        &HashMap::new(),
    );

    assert_eq!(results.len(), 1, "should find edgeworks_sdk");
    assert_eq!(results[0].lib_name, "edgeworks_sdk");
    assert!(
        results[0].protocols.contains(&"rest".to_string()),
        "edgeworks_sdk should be detected as wrapping rest via httpx"
    );
}

/// LRES-02: Node modules discovery — finds @acme/rpc in node_modules, detects axios wrapping
#[test]
fn test_lres02_node_modules_detection() {
    let dir = TempDir::new().unwrap();
    write_file(
        &dir,
        "node_modules/@acme/rpc/index.js",
        "const axios = require('axios');\nmodule.exports.call = (url) => axios.get(url);\n",
    );

    let registry = PatternRegistry::from_patterns(
        vec![Pattern {
            id: "ts-axios".to_string(),
            name: "axios".to_string(),
            description: "axios http client".to_string(),
            languages: vec!["typescript".to_string()],
            file_patterns: vec![],
            import_gate: vec!["axios".to_string()],
            detections: vec![Detection {
                match_str: "axios.get(".to_string(),
                kind: "connection".to_string(),
                protocol: "rest".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::FirstStringArg,
            }],
        }],
        "test-1.0".to_string(),
    );

    let mut resolver = LibraryResolver::new(dir.path());
    let results = resolver.resolve_for_language(
        &registry,
        "typescript",
        &["@acme/rpc".to_string()],
        &HashMap::new(),
    );

    assert!(!results.is_empty(), "should find @acme/rpc in node_modules");
    assert!(
        results[0].protocols.contains(&"rest".to_string()),
        "@acme/rpc should be detected as wrapping rest via axios"
    );
}

/// LRES-03: Lock file dependency resolution — Cargo.lock with tonic dep produces grpc
#[test]
fn test_lres03_cargo_lock_dep_resolution() {
    let dir = TempDir::new().unwrap();
    write_file(
        &dir,
        "Cargo.lock",
        "[[package]]\n\
         name = \"acme-rpc\"\n\
         version = \"0.1.0\"\n\
         dependencies = [\n\
           \"tonic 0.9.2\",\n\
           \"serde 1.0.0\",\n\
         ]\n\
         \n\
         [[package]]\n\
         name = \"tonic\"\n\
         version = \"0.9.2\"\n",
    );

    let resolver = LibraryResolver::new(dir.path());
    let lock_map = resolver.parse_cargo_lock();

    assert!(
        lock_map.contains_key("acme-rpc"),
        "Cargo.lock should contain acme-rpc"
    );
    let acme_deps = &lock_map["acme-rpc"];
    assert!(
        acme_deps.iter().any(|d| d == "tonic"),
        "acme-rpc deps should include tonic (version stripped)"
    );

    let protocols = infer_protocols_from_deps(acme_deps);
    assert!(
        protocols.contains(&"grpc".to_string()),
        "tonic dependency should infer grpc protocol"
    );
}

/// LRES-04: Cache prevents re-scanning — duplicate library in deps results in 1 entry and cache hit
#[test]
fn test_lres04_cache_prevents_rescan() {
    let dir = TempDir::new().unwrap();
    write_file(
        &dir,
        "venv/lib/python3.12/site-packages/edgeworks_sdk/client.py",
        "import httpx\n\ndef create_client(url):\n    return httpx.Client(url)\n",
    );

    let registry = make_httpx_registry();
    let mut resolver = LibraryResolver::new(dir.path());

    // First call — scans the library
    let results = resolver.resolve_for_language(
        &registry,
        "python",
        &["edgeworks_sdk".to_string()],
        &HashMap::new(),
    );

    assert_eq!(results.len(), 1, "should find edgeworks_sdk on first call");
    let first_protocols = results[0].protocols.clone();

    // Call again — should use cache (no re-scan)
    let results2 = resolver.resolve_for_language(
        &registry,
        "python",
        &["edgeworks_sdk".to_string()],
        &HashMap::new(),
    );

    assert_eq!(results2.len(), 1, "second call should return cached result");
    assert_eq!(
        results2[0].protocols, first_protocols,
        "cached protocols should match first call exactly"
    );
}

/// LRES-05: Missing environment — empty temp dir returns empty, no panic
#[test]
fn test_lres05_missing_env_continues() {
    let dir = TempDir::new().unwrap();
    // No venv, no node_modules, no vendor/bundle

    let registry = PatternRegistry::from_patterns(vec![], "test-1.0".to_string());
    let mut resolver = LibraryResolver::new(dir.path());

    // Should not panic, should return empty
    let results = resolver.resolve_for_language(
        &registry,
        "python",
        &["some_unknown_sdk".to_string()],
        &HashMap::new(),
    );

    assert!(
        results.is_empty(),
        "missing env should return empty results without panic (LRES-05)"
    );
}

/// LRES-06: Extraction method and confidence — verifies format and Confidence level
#[test]
fn test_lres06_extraction_method_and_confidence() {
    let dir = TempDir::new().unwrap();
    write_file(
        &dir,
        "venv/lib/python3.12/site-packages/edgeworks_sdk/client.py",
        "import httpx\n\ndef create_client(url):\n    return httpx.Client(url)\n",
    );

    let registry = make_httpx_registry();
    let mut resolver = LibraryResolver::new(dir.path());
    let results = resolver.resolve_for_language(
        &registry,
        "python",
        &["edgeworks_sdk".to_string()],
        &HashMap::new(),
    );

    assert_eq!(results.len(), 1);
    let resolved = &results[0];

    // Verify the extraction_method format that scanner.rs will use (LRES-06)
    // The → is Unicode U+2192 (RIGHT ARROW), not ASCII ->
    let expected_method = format!(
        "library_resolution:{}→{}",
        resolved.lib_name, resolved.protocols[0]
    );
    assert!(
        expected_method.starts_with("library_resolution:edgeworks_sdk→"),
        "extraction_method must use format library_resolution:{{lib}}→{{underlying}}"
    );
    assert!(
        expected_method.contains('\u{2192}'),
        "extraction_method must use Unicode → (U+2192), not ASCII ->"
    );

    // read_manifest_deps returns empty for missing manifest — does not panic
    let deps = read_manifest_deps(dir.path(), "python");
    assert!(
        deps.is_empty(),
        "no pyproject.toml or requirements.txt = empty dep list"
    );
}
