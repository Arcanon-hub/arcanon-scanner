//! v1.1 integration validation tests.
//!
//! Verifies that all five accuracy fixes from Phases 8 and 9 collectively
//! produce a false-positive-free scan result using the v1.1-validation fixture.
//!
//! Fixes covered:
//!   DACC-01: py-opcua import gate — no asyncua import → zero py-opcua connections
//!   DACC-04: Python docstring filter — CoreV1Api() inside docstrings must not fire
//!   DACC-05: py-kubernetes pattern — real CoreV1Api() call produces kubernetes connection
//!   DEBT-01: NestJS full paths — @Controller('/api/v1') + @Get('/users') → /api/v1/users
//!   DEBT-02: [services] config parsing — service-nestjs renamed to api-gateway

use arcanon::config::{DetectionOverride, PatternOverride};
use std::collections::HashMap;
use std::path::PathBuf;

fn fixture_path(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/v1.1-validation")
        .join(sub)
}

/// Build the py-kubernetes pattern as a PatternOverride so the integration test
/// is self-contained even when the remote CDN doesn't yet include this pattern.
/// This mirrors what a user would put in [[patterns]] in .arcanon.toml.
fn py_kubernetes_pattern() -> PatternOverride {
    PatternOverride {
        id: "py-kubernetes".to_string(),
        name: "py-kubernetes".to_string(),
        description: "Kubernetes Python client API instantiation".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec!["kubernetes".to_string()],
        detections: vec![
            DetectionOverride {
                match_str: "CoreV1Api(".to_string(),
                kind: "connection".to_string(),
                protocol: "kubernetes".to_string(),
                confidence: "high".to_string(),
                target_extraction: "none".to_string(),
            },
            DetectionOverride {
                match_str: "AppsV1Api(".to_string(),
                kind: "connection".to_string(),
                protocol: "kubernetes".to_string(),
                confidence: "high".to_string(),
                target_extraction: "none".to_string(),
            },
        ],
    }
}

fn make_config(root: PathBuf) -> arcanon::core::scanner::ScannerConfig {
    arcanon::core::scanner::ScannerConfig {
        root,
        dry_run: true,
        hub_url: "https://hub.example.com".to_string(),
        api_key: "test-key".to_string(),
        project_slug: "v1.1-validation".to_string(),
        output: None,
        plugin_filter: None,
        exclude_patterns: vec![],
        service_overrides: HashMap::new(),
        git_overrides: arcanon::core::scanner::GitOverrides::default(),
        user_pattern_overrides: vec![],
        disabled_patterns: vec![],
    }
}

/// DACC-01: service-opcua contains Client( but no asyncua import.
/// The import gate must block py-opcua — zero opcua connections expected.
#[test]
fn opcua_no_false_positive_without_import() {
    let root = fixture_path("service-opcua");
    let config = make_config(root);

    let payload = {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(arcanon::core::scanner::run(&config))
            .expect("scan must not fail")
    };

    eprintln!("[DACC-01] Services: {}", payload.findings.services.len());
    eprintln!(
        "[DACC-01] Total connections: {}",
        payload.findings.connections.len()
    );
    for conn in &payload.findings.connections {
        eprintln!(
            "  connection: protocol={} source={} file={} evidence={:?}",
            conn.protocol, conn.source, conn.source_file, conn.evidence
        );
    }

    // No asyncua import in service-opcua → zero py-opcua / opcua connections
    let opcua_conns: Vec<_> = payload
        .findings
        .connections
        .iter()
        .filter(|c| c.protocol.contains("opcua") || c.protocol.contains("opc"))
        .collect();

    assert_eq!(
        opcua_conns.len(),
        0,
        "DACC-01: service-opcua has no asyncua import — expected 0 opcua connections, got {}: {:?}",
        opcua_conns.len(),
        opcua_conns
            .iter()
            .map(|c| (&c.protocol, &c.source_file))
            .collect::<Vec<_>>()
    );
}

/// DACC-04 + DACC-05 + DACC-01 positive case:
/// service-fastapi has:
///   - Triple-quoted docstrings containing CoreV1Api() and Client() → must NOT fire
///   - A real k8s_client.CoreV1Api() call → must produce at least 1 kubernetes connection
///   - A real asyncua.Client() call → must produce at least 1 opcua connection
///
/// py-kubernetes is injected via user_pattern_overrides to make this test self-contained
/// regardless of whether the remote CDN has the pattern cached.
///
/// The key DACC-04 assertion: connections whose evidence text originates inside a docstring
/// must NOT appear. Specifically, "CoreV1Api() for pod management" is inside a docstring
/// and must not produce a kubernetes connection; "Client(\"opc.tcp://example:4840\")" is
/// inside a docstring and must not produce an opcua connection.
#[test]
fn fastapi_docstring_and_kubernetes() {
    let root = fixture_path("service-fastapi");
    let config = arcanon::core::scanner::ScannerConfig {
        root,
        dry_run: true,
        hub_url: "https://hub.example.com".to_string(),
        api_key: "test-key".to_string(),
        project_slug: "v1.1-validation".to_string(),
        output: None,
        plugin_filter: None,
        exclude_patterns: vec![],
        service_overrides: HashMap::new(),
        git_overrides: arcanon::core::scanner::GitOverrides::default(),
        user_pattern_overrides: vec![py_kubernetes_pattern()],
        disabled_patterns: vec![],
    };

    let payload = {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(arcanon::core::scanner::run(&config))
            .expect("scan must not fail")
    };

    eprintln!(
        "[DACC-04/05] Total connections: {}",
        payload.findings.connections.len()
    );
    for conn in &payload.findings.connections {
        eprintln!(
            "  connection: protocol={} source={} file={} evidence={:?}",
            conn.protocol, conn.source, conn.source_file, conn.evidence
        );
    }

    let opcua_conns: Vec<_> = payload
        .findings
        .connections
        .iter()
        .filter(|c| c.protocol.contains("opcua") || c.protocol.contains("opc"))
        .collect();

    let k8s_conns: Vec<_> = payload
        .findings
        .connections
        .iter()
        .filter(|c| c.protocol.contains("kubernetes") || c.protocol.contains("k8s"))
        .collect();

    // DACC-04: docstring text must NOT appear as connection evidence
    // "CoreV1Api() for pod management" is inside the DOCSTRING_DECOY triple-quoted block
    let docstring_k8s_false_positive = payload.findings.connections.iter().find(|c| {
        c.evidence
            .as_deref()
            .unwrap_or("")
            .contains("CoreV1Api() for pod management")
    });
    assert!(
        docstring_k8s_false_positive.is_none(),
        "DACC-04: 'CoreV1Api() for pod management' is inside a docstring and must not produce a connection. \
         Found: {:?}",
        docstring_k8s_false_positive
    );

    // DACC-04: opcua docstring text must not appear
    // "Client(\"opc.tcp://example:4840\")" is inside the DOCSTRING_DECOY block
    let docstring_opcua_false_positive = payload.findings.connections.iter().find(|c| {
        c.evidence
            .as_deref()
            .unwrap_or("")
            .contains("opc.tcp://example:4840")
    });
    assert!(
        docstring_opcua_false_positive.is_none(),
        "DACC-04: Client() inside docstring must not produce a connection. Found: {:?}",
        docstring_opcua_false_positive
    );

    // DACC-01 positive case: real asyncua.Client() call must fire
    assert!(
        !opcua_conns.is_empty(),
        "DACC-01: service-fastapi imports asyncua and calls asyncua.Client() — \
         expected >=1 opcua connection, got 0"
    );

    // DACC-05: real CoreV1Api() call must fire (py-kubernetes injected via user_pattern_overrides)
    assert!(
        !k8s_conns.is_empty(),
        "DACC-05: service-fastapi calls k8s_client.CoreV1Api() — \
         expected >=1 kubernetes connection, got 0. \
         Connections found: {:?}",
        payload
            .findings
            .connections
            .iter()
            .map(|c| (&c.protocol, &c.source_file))
            .collect::<Vec<_>>()
    );
}

/// DEBT-01: NestJS @Controller('/api/v1') + @Get('/users') must produce
/// endpoint with path "/api/v1/users", not just "/users" or bare method name.
#[test]
fn nestjs_full_paths() {
    let root = fixture_path("service-nestjs");
    let config = make_config(root);

    let payload = {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(arcanon::core::scanner::run(&config))
            .expect("scan must not fail")
    };

    let all_endpoints: Vec<_> = payload
        .findings
        .services
        .iter()
        .flat_map(|s| s.exposes.iter())
        .collect();

    eprintln!("[DEBT-01] Endpoints found: {}", all_endpoints.len());
    for ep in &all_endpoints {
        eprintln!("  {} {}", ep.method, ep.path);
    }

    let full_path_ep = all_endpoints
        .iter()
        .find(|e| e.path == "/api/v1/users" && e.method == "GET");

    assert!(
        full_path_ep.is_some(),
        "DEBT-01: Expected GET /api/v1/users (full controller prefix + method path). \
         Endpoints found: {:?}",
        all_endpoints
            .iter()
            .map(|e| format!("{} {}", e.method, e.path))
            .collect::<Vec<_>>()
    );
}

/// DEBT-02 + full scan: scans entire v1.1-validation fixture with .arcanon.toml loaded.
///
/// Asserts:
///   - "api-gateway" present (service-nestjs renamed via [services])
///   - "service-ignored" absent (ignore = true)
///   - total connections >= 2 (service-fastapi contributes opcua + kubernetes; service-opcua contributes 0)
///   - payload serializes to valid JSON containing "version":"1.0"
///   - files_scanned >= 5
#[test]
fn full_fixture_scan_with_arcanon_toml() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v1.1-validation");

    // Load .arcanon.toml and bridge ServiceConfig → ServiceOverride (mirrors main.rs)
    let file_cfg = arcanon::config::load_file_config(&root);
    let service_overrides: HashMap<String, arcanon::core::merger::ServiceOverride> = file_cfg
        .services
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                arcanon::core::merger::ServiceOverride {
                    name: v.name,
                    ignore: v.ignore,
                },
            )
        })
        .collect();

    eprintln!(
        "[DEBT-02] Loaded {} service overrides from .arcanon.toml",
        service_overrides.len()
    );

    let config = arcanon::core::scanner::ScannerConfig {
        root,
        dry_run: true,
        hub_url: "https://hub.example.com".to_string(),
        api_key: "test-key".to_string(),
        project_slug: "v1.1-validation".to_string(),
        output: None,
        plugin_filter: None,
        exclude_patterns: vec![],
        service_overrides,
        git_overrides: arcanon::core::scanner::GitOverrides::default(),
        // Inject py-kubernetes so the full scan also finds kubernetes connections
        user_pattern_overrides: vec![py_kubernetes_pattern()],
        disabled_patterns: vec![],
    };

    let payload = {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(arcanon::core::scanner::run(&config))
            .expect("scan must not fail")
    };

    eprintln!(
        "[DEBT-02] Services detected: {}",
        payload.findings.services.len()
    );
    for svc in &payload.findings.services {
        eprintln!(
            "  - {} (root_path={}, endpoints={})",
            svc.name,
            svc.root_path,
            svc.exposes.len()
        );
    }
    eprintln!(
        "[DEBT-02] Total connections: {}",
        payload.findings.connections.len()
    );
    for conn in &payload.findings.connections {
        eprintln!(
            "  connection: protocol={} source={} file={}",
            conn.protocol, conn.source, conn.source_file
        );
    }

    let service_names: Vec<&str> = payload
        .findings
        .services
        .iter()
        .map(|s| s.name.as_str())
        .collect();

    // DEBT-02: service-nestjs renamed to api-gateway
    assert!(
        service_names.contains(&"api-gateway"),
        "DEBT-02: Expected 'api-gateway' (renamed from service-nestjs). \
         Services found: {:?}",
        service_names
    );

    // DEBT-02: service-ignored must not appear (ignore = true)
    assert!(
        !service_names.contains(&"service-ignored"),
        "DEBT-02: 'service-ignored' must be absent (ignore = true). \
         Services found: {:?}",
        service_names
    );

    // service-opcua contributes 0 connections (no asyncua import)
    // service-fastapi contributes >=2 connections (opcua + kubernetes)
    assert!(
        payload.findings.connections.len() >= 2,
        "Expected >= 2 connections (from service-fastapi), got {}",
        payload.findings.connections.len()
    );

    // DACC-01: service-opcua contributes zero opcua connections
    let opcua_from_opcua_svc: Vec<_> = payload
        .findings
        .connections
        .iter()
        .filter(|c| {
            (c.protocol.contains("opcua") || c.protocol.contains("opc"))
                && (c.source_file.contains("service-opcua") || c.source.contains("service-opcua"))
        })
        .collect();
    assert_eq!(
        opcua_from_opcua_svc.len(),
        0,
        "DACC-01: service-opcua must contribute 0 opcua connections (no asyncua import). \
         Got: {:?}",
        opcua_from_opcua_svc
            .iter()
            .map(|c| (&c.source_file, &c.evidence))
            .collect::<Vec<_>>()
    );

    // Payload serializes to valid JSON with version 1.0
    let json = serde_json::to_string(&payload).expect("payload must serialize to JSON");
    assert!(
        json.contains("\"version\":\"1.0\""),
        "Payload JSON must contain version 1.0"
    );

    // Files scanned sanity check (at least .py, .ts, Dockerfiles, requirements)
    assert!(
        payload.metadata.files_scanned >= 5,
        "Expected >= 5 files scanned, got {}",
        payload.metadata.files_scanned
    );
}
