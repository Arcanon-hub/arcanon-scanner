//! Integration tests for wrapper tracing (Phase 7).
//! One test per WRAP requirement. All tests use synthetic in-memory fixtures.

use arcanon::patterns::{Detection, Pattern, PatternConfidence, PatternRegistry, TargetExtraction};
use arcanon::plugin::FileContext;
use arcanon::wrapper::{build_wrapper_map, detect_wrapper_calls, normalize_template_literal};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Helper: create an in-memory FileContext
fn make_file(relative: &str, content: &str) -> FileContext {
    FileContext {
        path: PathBuf::from("/repo").join(relative),
        relative_path: relative.to_string(),
        content: Arc::from(content),
    }
}

/// Helper: build a PatternRegistry seeded with "fetch" → rest
fn make_registry_with_fetch() -> PatternRegistry {
    PatternRegistry::from_patterns(
        vec![Pattern {
            id: "fetch-rest".to_string(),
            name: "fetch".to_string(),
            description: "Browser fetch API".to_string(),
            languages: vec!["typescript".to_string()],
            file_patterns: vec![],
            import_gate: vec![],
            detections: vec![Detection {
                match_str: "fetch(".to_string(),
                kind: "connection".to_string(),
                protocol: "rest".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::FirstStringArg,
            }],
        }],
        "test-1.0".to_string(),
    )
}

/// WRAP-01: Pass 1 finds `function apiFetch(path) { fetch(...) }` and marks apiFetch as a fetch wrapper
#[test]
fn test_wrap_01_pass1_finds_user_code_wrapper() {
    let registry = make_registry_with_fetch();

    let api_file = make_file(
        "src/lib/api.ts",
        r#"
export function apiFetch(path: string, opts?: RequestInit) {
    return fetch(`${window.API_BASE}${path}`, {
        headers: { Authorization: `Bearer ${getToken()}` },
        ...opts,
    });
}
"#,
    );

    let map = build_wrapper_map(&[api_file], &[], &registry, "typescript");

    assert!(
        map.contains("apiFetch"),
        "Pass 1 should have found apiFetch as a fetch wrapper"
    );
    let info = map.get("apiFetch").unwrap();
    assert_eq!(info.protocol, "rest");
    assert!(
        info.chain.contains(&"fetch".to_string()),
        "chain should include 'fetch': {:?}",
        info.chain
    );
    assert_eq!(
        info.depth, 1,
        "apiFetch wraps fetch directly — depth should be 1"
    );
}

/// WRAP-02: Pass 2 detects `apiFetch('/api/v1/teams')` and extracts `/api/v1/teams` as the path
#[test]
fn test_wrap_02_pass2_detects_wrapper_call_with_path() {
    let registry = make_registry_with_fetch();

    let api_file = make_file(
        "src/lib/api.ts",
        "export function apiFetch(path: string) { return fetch(path); }",
    );

    // Separate file for the caller to avoid wrapper detection of the function declaration itself
    let caller_file = make_file(
        "src/hooks/useTeams.ts",
        r#"import { apiFetch } from '../lib/api';

const result = apiFetch('/api/v1/teams');
"#,
    );

    let all_files = vec![api_file, caller_file.clone()];
    let map = build_wrapper_map(&all_files, &[], &registry, "typescript");

    assert!(
        map.contains("apiFetch"),
        "wrapper map must contain apiFetch"
    );

    let result = detect_wrapper_calls(&[caller_file], &map, &HashMap::new());

    // Find the connection with a path (skip any false positives from function declarations)
    let conn = result
        .connections
        .iter()
        .find(|c| c.path.is_some())
        .expect("Pass 2 should detect the apiFetch call with a path");

    assert_eq!(conn.protocol, "rest");
    assert_eq!(
        conn.path,
        Some("/api/v1/teams".to_string()),
        "path should be extracted from string literal"
    );
    assert!(
        conn.source_file.contains("useTeams.ts"),
        "source_file should reference the caller file: {}",
        conn.source_file
    );
}

/// WRAP-03: Library wrapper — scanned as lib_files, ends up in same wrapper map
#[test]
fn test_wrap_03_library_wrapper_detection() {
    let registry = make_registry_with_fetch();

    // Simulate an installed library source file
    let lib_file = make_file(
        "node_modules/@acme/rpc/src/client.ts",
        r#"export class RpcClient {
    post(path: string, body: unknown) {
        return fetch(path, { method: 'POST', body: JSON.stringify(body) });
    }
}
"#,
    );

    // lib_files: (lib_name, Vec<FileContext>)
    let lib_files = vec![("@acme/rpc".to_string(), vec![lib_file])];

    let map = build_wrapper_map(&[], &lib_files, &registry, "typescript");

    assert!(
        map.contains("post"),
        "library method 'post' should be detected as a REST wrapper. Map contains: {:?}",
        map.iter().map(|(k, _)| k).collect::<Vec<_>>()
    );
    let info = map.get("post").unwrap();
    assert_eq!(info.protocol, "rest");
}

/// WRAP-04: Template literal `/api/v1/orgs/${orgId}/teams` → `/api/v1/orgs/{param}/teams`
#[test]
fn test_wrap_04_template_literal_normalization() {
    // Direct unit test of normalize_template_literal
    assert_eq!(
        normalize_template_literal("`/api/v1/orgs/${orgId}/teams`"),
        "/api/v1/orgs/{param}/teams"
    );
    assert_eq!(
        normalize_template_literal("f\"/api/{org_id}/items\""),
        "/api/{param}/items"
    );
    assert_eq!(
        normalize_template_literal("\"/api/%s/items\""),
        "/api/{param}/items"
    );
    assert_eq!(
        normalize_template_literal("\"/api/#{org_id}/items\""),
        "/api/{param}/items"
    );

    // Integration test: template literal in a wrapper call is normalized in ConnectionInfo.path
    let registry = make_registry_with_fetch();
    let api_file = make_file(
        "src/lib/api.ts",
        "export function apiFetch(path: string) { return fetch(path); }",
    );
    let caller_file = make_file(
        "src/hooks/useOrg.ts",
        r#"import { apiFetch } from '../lib/api';

const result = apiFetch(`/api/v1/orgs/${orgId}/teams`);
"#,
    );
    let all_files = vec![api_file, caller_file.clone()];
    let map = build_wrapper_map(&all_files, &[], &registry, "typescript");
    let result = detect_wrapper_calls(&[caller_file], &map, &HashMap::new());

    assert!(!result.connections.is_empty());
    // Find the connection with a path
    let conn = result
        .connections
        .iter()
        .find(|c| c.path.is_some())
        .expect("Should find connection with normalized path");
    assert_eq!(
        conn.path,
        Some("/api/v1/orgs/{param}/teams".to_string()),
        "template literal should be normalized: {:?}",
        conn.path
    );
}

/// WRAP-05: Wrapper chains — useData → apiFetch → fetch results in useData at depth 2
#[test]
fn test_wrap_05_wrapper_chain_multi_level() {
    let registry = make_registry_with_fetch();

    let api_file = make_file(
        "src/lib/api.ts",
        "export function apiFetch(path: string) { return fetch(path); }",
    );
    let data_file = make_file(
        "src/hooks/useData.ts",
        "import { apiFetch } from '../lib/api';\nexport function useData(path: string) { return apiFetch(path); }",
    );

    let all_files = vec![api_file, data_file];
    let map = build_wrapper_map(&all_files, &[], &registry, "typescript");

    assert!(
        map.contains("apiFetch"),
        "apiFetch must be in map (level 1)"
    );
    assert!(map.contains("useData"), "useData must be in map (level 2)");

    let use_data_info = map.get("useData").unwrap();
    assert_eq!(use_data_info.protocol, "rest");
    assert_eq!(use_data_info.depth, 2, "useData is 2 levels deep");
    assert!(
        use_data_info.chain.contains(&"fetch".to_string()),
        "chain must include terminal 'fetch': {:?}",
        use_data_info.chain
    );
}

/// WRAP-06: Wrapper map is returned from build_wrapper_map and reused (per-scan cache)
#[test]
fn test_wrap_06_wrapper_map_reused_across_detect_calls() {
    let registry = make_registry_with_fetch();

    let api_file = make_file(
        "src/lib/api.ts",
        "export function apiFetch(path: string) { return fetch(path); }",
    );
    let file_a = make_file(
        "src/components/A.ts",
        "import { apiFetch } from '../lib/api';\nconst a = apiFetch('/api/a');",
    );
    let file_b = make_file(
        "src/components/B.ts",
        "import { apiFetch } from '../lib/api';\nconst b = apiFetch('/api/b');",
    );

    // Build map once (simulating per-scan cache — D-06)
    let all_files = vec![api_file];
    let map = build_wrapper_map(&all_files, &[], &registry, "typescript");

    // Use the same map for multiple detect calls
    let result_a = detect_wrapper_calls(&[file_a], &map, &HashMap::new());
    let result_b = detect_wrapper_calls(&[file_b], &map, &HashMap::new());

    assert_eq!(
        result_a.connections.len(),
        1,
        "File A should detect one wrapper call"
    );
    assert_eq!(
        result_b.connections.len(),
        1,
        "File B should detect one wrapper call"
    );
    assert_eq!(result_a.connections[0].path, Some("/api/a".to_string()));
    assert_eq!(result_b.connections[0].path, Some("/api/b".to_string()));
}

/// WRAP-07: extraction_method is "wrapper_trace:{wrapper}→{terminal}"
#[test]
fn test_wrap_07_extraction_method_format() {
    let registry = make_registry_with_fetch();

    let api_file = make_file(
        "src/lib/api.ts",
        "export function apiFetch(path: string) { return fetch(path); }",
    );
    let caller_file = make_file(
        "src/app.ts",
        "import { apiFetch } from './lib/api';\napiFetch('/health');",
    );

    let all_files = vec![api_file, caller_file.clone()];
    let map = build_wrapper_map(&all_files, &[], &registry, "typescript");
    let result = detect_wrapper_calls(&[caller_file], &map, &HashMap::new());

    assert!(!result.connections.is_empty());
    let conn = &result.connections[0];

    // extraction_method must be "wrapper_trace:{wrapper}→{terminal}"
    assert_eq!(
        conn.extraction_method, "wrapper_trace:apiFetch→fetch",
        "extraction_method should follow the required format"
    );
}
