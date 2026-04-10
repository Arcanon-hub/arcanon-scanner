//! Integration test for the polyglot fixture end-to-end scanning.
//! Verifies that the scanner correctly detects services, endpoints, and connections
//! from a mixed-language monorepo (NestJS + FastAPI + unscoped shared library).

use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn test_polyglot_fixture_end_to_end() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polyglot");

    let config = arcanon::core::scanner::ScannerConfig {
        root: fixture_root,
        dry_run: true,
        hub_url: "https://hub.example.com".to_string(),
        api_key: "test-key".to_string(),
        project_slug: "test".to_string(),
        output: None,
        plugin_filter: None,
        exclude_patterns: vec![],
        service_overrides: HashMap::new(),
        git_overrides: arcanon::core::scanner::GitOverrides::default(),
        user_pattern_overrides: vec![],
        disabled_patterns: vec![],
        ..Default::default()
    };

    let payload = {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(arcanon::core::scanner::run(&config))
            .expect("scan should succeed")
    };

    // Debug output
    eprintln!("Services detected: {}", payload.findings.services.len());
    for svc in &payload.findings.services {
        eprintln!("  - {} (endpoints: {})", svc.name, svc.exposes.len());
        for ep in &svc.exposes {
            eprintln!("    - {} {}", ep.method, ep.path);
        }
    }
    eprintln!("Total connections: {}", payload.findings.connections.len());

    // MONO-01/02: exactly 2 services detected (from 2 Dockerfiles)
    assert_eq!(
        payload.findings.services.len(),
        2,
        "Expected 2 services, got {}: {:?}",
        payload.findings.services.len(),
        payload
            .findings
            .services
            .iter()
            .map(|s| &s.name)
            .collect::<Vec<_>>()
    );

    // Service names contain service-a and service-b
    let names: Vec<&str> = payload
        .findings
        .services
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert!(
        names.iter().any(|n| n.contains("service-a")),
        "service-a not found in: {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n.contains("service-b")),
        "service-b not found in: {:?}",
        names
    );

    // Count total endpoints
    let all_endpoints: Vec<_> = payload
        .findings
        .services
        .iter()
        .flat_map(|s| s.exposes.iter())
        .collect();

    // DETQ-05/DEBT-01: NestJS two-phase extraction must produce GET /users/:id
    let nestjs_endpoint = all_endpoints
        .iter()
        .find(|e| e.path == "/users/:id" && e.method == "GET");
    assert!(
        nestjs_endpoint.is_some(),
        "NestJS GET /users/:id not found. Endpoints: {:?}",
        all_endpoints
            .iter()
            .map(|e| (&e.method, &e.path))
            .collect::<Vec<_>>()
    );

    // LPLU-02: FastAPI endpoint — GET /items
    let fastapi_endpoint = all_endpoints
        .iter()
        .find(|e| e.path == "/items" && e.method == "GET");
    assert!(
        fastapi_endpoint.is_some(),
        "FastAPI GET /items not found in endpoints: {:?}",
        all_endpoints
            .iter()
            .map(|e| (&e.method, &e.path))
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_polyglot_fixture_files_exist() {
    // Quick sanity check that fixtures were created
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polyglot");
    assert!(
        root.join("service-a/Dockerfile").exists(),
        "service-a/Dockerfile must exist"
    );
    assert!(
        root.join("service-a/package.json").exists(),
        "service-a/package.json must exist"
    );
    assert!(
        root.join("service-a/src/users.ts").exists(),
        "service-a/src/users.ts must exist"
    );
    assert!(
        root.join("service-b/Dockerfile").exists(),
        "service-b/Dockerfile must exist"
    );
    assert!(
        root.join("service-b/app.py").exists(),
        "service-b/app.py must exist"
    );
    assert!(
        root.join("lib/shared.ts").exists(),
        "lib/shared.ts must exist"
    );
    // lib/ must NOT have a Dockerfile (MONO-03 test requirement)
    assert!(
        !root.join("lib/Dockerfile").exists(),
        "lib/ must NOT have Dockerfile"
    );
}

/// Diagnostic test: directly verify TypeScript plugin extracts NestJS routes from fixture files.
/// This isolates the extraction from the full scanner pipeline.
#[test]
fn test_nestjs_extraction_from_fixture() {
    use arcanon::plugin::{ExtractionContext, FileContext, LanguagePlugin};
    use std::collections::HashMap;
    use std::sync::Arc;

    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polyglot");
    let pkg_path = fixture_root.join("service-a/package.json");
    let ts_path = fixture_root.join("service-a/src/users.ts");

    let pkg_content = std::fs::read_to_string(&pkg_path).expect("package.json must be readable");
    let ts_content = std::fs::read_to_string(&ts_path).expect("users.ts must be readable");

    let pkg_file = FileContext {
        path: pkg_path.clone(),
        relative_path: "service-a/package.json".to_string(),
        content: Arc::from(pkg_content.as_str()),
    };
    let ts_file = FileContext {
        path: ts_path.clone(),
        relative_path: "service-a/src/users.ts".to_string(),
        content: Arc::from(ts_content.as_str()),
    };

    let mut service_roots = HashMap::new();
    service_roots.insert(fixture_root.join("service-a"), "service-a".to_string());

    let ctx = ExtractionContext {
        files: vec![pkg_file, ts_file],
        vars: Arc::new(arcanon::vars::VariableStore::new()),
        root: fixture_root,
        service_roots,
    };

    let plugin = arcanon::plugin::lang::typescript::TypeScriptPlugin;
    let result = plugin.extract(&ctx);

    eprintln!("Endpoints from fixture: {}", result.endpoints.len());
    for ep in &result.endpoints {
        eprintln!("  {} {} (service={})", ep.method, ep.path, ep.service_name);
    }

    let get_ep = result
        .endpoints
        .iter()
        .find(|e| e.method == "GET" && e.path == "/users/:id");
    assert!(
        get_ep.is_some(),
        "Expected GET /users/:id from fixture. Got: {:?}",
        result
            .endpoints
            .iter()
            .map(|e| (&e.method, &e.path))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        get_ep.unwrap().service_name,
        "service-a",
        "Endpoint must be scoped to service-a"
    );
}

/// Diagnostic: verify walk_repo finds the TypeScript files in the polyglot fixture.
#[test]
fn test_walk_repo_finds_ts_files() {
    use arcanon::discovery::walk_repo;

    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polyglot");

    let files = walk_repo(&fixture_root, &[]).expect("walk_repo should succeed");
    let relative: Vec<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(&fixture_root)
                .unwrap_or(p)
                .to_string_lossy()
                .to_string()
        })
        .collect();

    eprintln!("Files found by walk_repo: {:?}", relative);

    let has_users_ts = relative.iter().any(|p| p.contains("users.ts"));
    assert!(
        has_users_ts,
        "walk_repo must find service-a/src/users.ts. Found: {:?}",
        relative
    );
}
