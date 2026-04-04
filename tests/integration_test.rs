//! Integration test for the polyglot fixture end-to-end scanning.
//! Verifies that the scanner correctly detects services, endpoints, and connections
//! from a mixed-language monorepo (NestJS + FastAPI + unscoped shared library).

use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn test_polyglot_fixture_end_to_end() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/polyglot");

    let config = arcanon_scanner::core::scanner::ScannerConfig {
        root: fixture_root,
        dry_run: true,
        hub_url: "https://hub.example.com".to_string(),
        api_key: "test-key".to_string(),
        project_slug: "test".to_string(),
        output: None,
        plugin_filter: None,
        exclude_patterns: vec![],
        service_overrides: HashMap::new(),
        git_overrides: arcanon_scanner::core::scanner::GitOverrides::default(),
        user_pattern_overrides: vec![],
        disabled_patterns: vec![],
    };

    let payload = {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(arcanon_scanner::core::scanner::run(&config))
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

    // DETQ-05: NestJS two-phase endpoint — GET /users/:id
    let nestjs_endpoint = all_endpoints
        .iter()
        .find(|e| e.path == "/users/:id" && e.method == "GET");
    assert!(
        nestjs_endpoint.is_some(),
        "NestJS GET /users/:id not found in: {:?}",
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
