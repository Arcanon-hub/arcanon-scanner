//! End-to-end test that scans the e2e fixture directory.
//! Verifies that the full pipeline produces a valid ScanPayloadV1 with expected structure.

use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn e2e_scan_fixture_repo_produces_valid_payload() {
    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/e2e");

    assert!(
        fixture_root.exists(),
        "E2E fixture directory must exist at {}",
        fixture_root.display()
    );

    // Build a minimal ScanConfig pointing at fixture_root
    let scan_config = arcanon::core::scanner::ScannerConfig {
        root: fixture_root,
        dry_run: true, // don't actually upload
        output: None,
        hub_url: "https://hub.example.com".to_string(),
        api_key: "test-key".to_string(),
        project_slug: "e2e-test".to_string(),
        plugin_filter: None,
        exclude_patterns: vec![],
        service_overrides: HashMap::new(),
        git_overrides: arcanon::core::scanner::GitOverrides::default(),
        user_pattern_overrides: vec![],
        disabled_patterns: vec![],
    };

    let payload = {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(arcanon::core::scanner::run(&scan_config))
            .expect("Scanner should not fail on valid fixture directory")
    };

    // Debug: print what was detected
    eprintln!("Services detected: {}", payload.findings.services.len());
    for svc in &payload.findings.services {
        eprintln!("  - {} (endpoints: {})", svc.name, svc.exposes.len());
        for ep in &svc.exposes {
            eprintln!("    - {} {}", ep.method, ep.path);
        }
    }
    eprintln!("Total connections: {}", payload.findings.connections.len());

    // Verify at least one service detected (from Dockerfile/compose)
    assert!(
        !payload.findings.services.is_empty(),
        "Expected at least one service from Dockerfile/compose, got 0"
    );

    // Verify at least one endpoint (from openapi.yaml)
    let total_endpoints: usize = payload
        .findings
        .services
        .iter()
        .map(|s| s.exposes.len())
        .sum();
    assert!(
        total_endpoints >= 2,
        "Expected at least 2 endpoints from openapi.yaml, got {}",
        total_endpoints
    );

    // Verify payload serializes to valid JSON
    let json = serde_json::to_string(&payload).expect("Payload must serialize to valid JSON");
    assert!(
        json.contains("\"version\":\"1.0\""),
        "Payload JSON must contain version 1.0"
    );
    assert!(
        json.contains("\"tool\":\"cli\""),
        "Payload JSON must contain tool=cli"
    );

    // Verify file count
    assert!(
        payload.metadata.files_scanned >= 1,
        "Expected at least 1 file scanned"
    );
}
