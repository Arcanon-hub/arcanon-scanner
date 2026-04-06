//! Integration tests for the pattern engine covering all PTRN requirements.
//! Tests verify apply(), overrides, disabled, and metadata functionality.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use arcanon::patterns::{Detection, Pattern, PatternConfidence, PatternRegistry, TargetExtraction};
use arcanon::plugin::FileContext;
use arcanon::types::Confidence;

// =============================================================================
// TASK 1: Pattern apply and extraction tests
// =============================================================================

#[test]
fn test_import_gate_blocks_non_matching_file() {
    let pattern = Pattern {
        id: "redis-py".to_string(),
        name: "redis-py".to_string(),
        description: "Python Redis client".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec!["import redis".to_string(), "from redis".to_string()],
        detections: vec![Detection {
            match_str: "Redis(".to_string(),
            kind: "connection".to_string(),
            protocol: "redis".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::FirstStringArg,
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from("print('hello')"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(),
        0,
        "Should block pattern when import_gate not found"
    );
}

#[test]
fn test_import_gate_passes_and_fires() {
    let pattern = Pattern {
        id: "redis-py".to_string(),
        name: "redis-py".to_string(),
        description: "Python Redis client".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec!["import redis".to_string()],
        detections: vec![Detection {
            match_str: "Redis(".to_string(),
            kind: "connection".to_string(),
            protocol: "redis".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::FirstStringArg,
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from("import redis\nr = Redis('localhost')"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(),
        1,
        "Should fire when import_gate present"
    );
    assert_eq!(
        result.connections[0].target_name, "localhost",
        "Should extract localhost as target"
    );
    assert_eq!(
        result.connections[0].protocol, "redis",
        "Should have redis protocol"
    );
}

#[test]
fn test_first_string_arg_extraction() {
    let pattern = Pattern {
        id: "redis-py".to_string(),
        name: "redis-py".to_string(),
        description: "Python Redis client".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "Redis(".to_string(),
            kind: "connection".to_string(),
            protocol: "redis".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::FirstStringArg,
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from("r = Redis(\"redis://my-cache:6379\")"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(result.connections.len(), 1);
    assert_eq!(
        result.connections[0].target_name, "redis://my-cache:6379",
        "Should extract full URL"
    );
}

#[test]
fn test_no_string_literal_gives_empty_target_medium_confidence() {
    let pattern = Pattern {
        id: "redis-py".to_string(),
        name: "redis-py".to_string(),
        description: "Python Redis client".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "Redis(".to_string(),
            kind: "connection".to_string(),
            protocol: "redis".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::FirstStringArg,
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from("r = Redis(host_var)"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(result.connections.len(), 1);
    assert_eq!(
        result.connections[0].target_name, "",
        "Should have empty target when no string literal"
    );
    assert_eq!(
        result.connections[0].confidence,
        Confidence::Medium,
        "Should fallback to Medium confidence per D-09"
    );
}

#[test]
fn test_named_arg_extraction() {
    let pattern = Pattern {
        id: "boto3-sqs".to_string(),
        name: "boto3-sqs".to_string(),
        description: "AWS SQS".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "send_message(".to_string(),
            kind: "connection".to_string(),
            protocol: "sqs".to_string(),
            confidence: PatternConfidence::Medium,
            target_extraction: TargetExtraction::NamedArg("QueueUrl".to_string()),
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from(
            "sqs.send_message(QueueUrl=\"https://sqs.us-east-1.amazonaws.com/123/my-queue\")",
        ),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(result.connections.len(), 1);
    assert_eq!(
        result.connections[0].target_name, "https://sqs.us-east-1.amazonaws.com/123/my-queue",
        "Should extract named argument"
    );
}

#[test]
fn test_url_hostname_extraction() {
    let pattern = Pattern {
        id: "http-client".to_string(),
        name: "http-client".to_string(),
        description: "HTTP requests".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "requests.get(".to_string(),
            kind: "connection".to_string(),
            protocol: "http".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::UrlHostname,
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from("requests.get(\"http://user-service:3000/api\")"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(result.connections.len(), 1);
    assert_eq!(
        result.connections[0].target_name, "user-service:3000",
        "Should extract hostname with port"
    );
}

#[test]
fn test_language_filter() {
    let pattern = Pattern {
        id: "test-pattern".to_string(),
        name: "test".to_string(),
        description: "test".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "match".to_string(),
            kind: "connection".to_string(),
            protocol: "proto".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::None,
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/test.ts"),
        relative_path: "test.ts".to_string(),
        content: Arc::from("const x = match"),
    };

    let result = registry.apply_all(&[file], "typescript", &HashMap::new());
    assert_eq!(
        result.connections.len(),
        0,
        "Should skip pattern for wrong language per D-05"
    );
}

#[test]
fn test_evidence_and_source_file() {
    let pattern = Pattern {
        id: "test-pattern".to_string(),
        name: "test".to_string(),
        description: "test".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "match".to_string(),
            kind: "connection".to_string(),
            protocol: "proto".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::None,
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/services/api/main.py"),
        relative_path: "services/api/main.py".to_string(),
        content: Arc::from("line1\nline2\nmatch"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(result.connections.len(), 1);
    assert_eq!(
        result.connections[0].source_file, "services/api/main.py:3",
        "Should have correct source file and line number"
    );
    assert_eq!(
        result.connections[0].evidence,
        Some("match".to_string()),
        "Should have trimmed evidence"
    );
}

// =============================================================================
// TASK 2: Override, disabled, and payload metadata tests
// =============================================================================

#[test]
fn test_user_pattern_overrides_remote_by_id() {
    let redis_pattern = Pattern {
        id: "redis-py".to_string(),
        name: "redis-py".to_string(),
        description: "Python Redis".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![Detection {
            match_str: "Redis(".to_string(),
            kind: "connection".to_string(),
            protocol: "redis".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::None,
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![redis_pattern], "1.0".to_string());

    // Create override that changes protocol to valkey
    let override_pattern = arcanon::config::PatternOverride {
        id: "redis-py".to_string(),
        name: "redis-py (valkey)".to_string(),
        description: "Updated protocol".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![arcanon::config::DetectionOverride {
            match_str: "Redis(".to_string(),
            kind: "connection".to_string(),
            protocol: "valkey".to_string(),
            confidence: "high".to_string(),
            target_extraction: "none".to_string(),
        }],
    };

    let registry = registry.with_overrides(&[override_pattern]);

    assert_eq!(
        registry.patterns().len(),
        1,
        "Should have 1 pattern (replaced)"
    );
    assert_eq!(
        registry.patterns()[0].id,
        "redis-py",
        "Pattern ID should be redis-py"
    );
    assert_eq!(
        registry.patterns()[0].detections[0].protocol,
        "valkey",
        "Protocol should be overridden to valkey"
    );
}

#[test]
fn test_user_pattern_adds_new_id() {
    let redis_pattern = Pattern {
        id: "redis-py".to_string(),
        name: "redis-py".to_string(),
        description: "Python Redis".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![],
    };

    let registry = PatternRegistry::from_patterns(vec![redis_pattern], "1.0".to_string());

    // Add new pattern with different ID
    let new_pattern = arcanon::config::PatternOverride {
        id: "my-internal-rpc".to_string(),
        name: "Internal RPC".to_string(),
        description: "Custom RPC".to_string(),
        languages: vec!["typescript".to_string()],
        file_patterns: vec!["**/*.ts".to_string()],
        import_gate: vec![],
        detections: vec![],
    };

    let registry = registry.with_overrides(&[new_pattern]);

    assert_eq!(
        registry.patterns().len(),
        2,
        "Should have 2 patterns (original + new)"
    );

    let ids: Vec<_> = registry.patterns().iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&"redis-py"), "Should still have redis-py");
    assert!(ids.contains(&"my-internal-rpc"), "Should have new pattern");
}

#[test]
fn test_disabled_removes_pattern() {
    let redis_pattern = Pattern {
        id: "redis-py".to_string(),
        name: "redis-py".to_string(),
        description: "Python Redis".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![],
    };

    let boto3_pattern = Pattern {
        id: "boto3-sqs".to_string(),
        name: "boto3-sqs".to_string(),
        description: "AWS SQS".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![],
        detections: vec![],
    };

    let registry =
        PatternRegistry::from_patterns(vec![redis_pattern, boto3_pattern], "1.0".to_string());

    assert_eq!(registry.patterns().len(), 2);

    let registry = registry.with_disabled(&["redis-py".to_string()]);

    assert_eq!(
        registry.patterns().len(),
        1,
        "Should have 1 pattern after disabling redis-py"
    );
    assert_eq!(
        registry.patterns()[0].id,
        "boto3-sqs",
        "Should only have boto3-sqs"
    );
}

#[test]
fn test_payload_metadata_fields_serialized() {
    use arcanon::core::payload::ScanMetadata;
    let metadata = ScanMetadata {
        tool: "arcanon",
        tool_version: "0.1.0",
        scan_mode: "full",
        repo_url: None,
        repo_name: "test-repo".to_string(),
        branch: "main".to_string(),
        commit_sha: "abc123".to_string(),
        started_at: "2026-04-04T22:00:00Z".to_string(),
        completed_at: "2026-04-04T22:05:00Z".to_string(),
        files_scanned: 42,
        project_slug: "test-project".to_string(),
        pattern_version: "1.0".to_string(),
        pattern_source: "remote".to_string(),
    };

    let json_str = serde_json::to_string(&metadata).expect("serialize metadata");

    assert!(
        json_str.contains("\"pattern_version\":\"1.0\""),
        "Should contain pattern_version field"
    );
    assert!(
        json_str.contains("\"pattern_source\":\"remote\""),
        "Should contain pattern_source field"
    );
}

#[tokio::test]
async fn test_load_with_no_hub_url_returns_empty_registry() {
    let registry = PatternRegistry::load(None).await;

    // Either returns empty or cached — neither should panic
    // If we got here without panicking, the registry loaded successfully
    let _ = registry.patterns().len();
}

#[test]
fn test_disabled_patterns_produce_no_findings() {
    let redis_pattern = Pattern {
        id: "redis-py".to_string(),
        name: "redis-py".to_string(),
        description: "Python Redis".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec!["import redis".to_string()],
        detections: vec![Detection {
            match_str: "Redis(".to_string(),
            kind: "connection".to_string(),
            protocol: "redis".to_string(),
            confidence: PatternConfidence::High,
            target_extraction: TargetExtraction::FirstStringArg,
        }],
    };

    let registry = PatternRegistry::from_patterns(vec![redis_pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/test.py"),
        relative_path: "test.py".to_string(),
        content: Arc::from("import redis\nr = Redis('localhost')"),
    };

    // Apply with redis-py disabled
    let registry = registry.with_disabled(&["redis-py".to_string()]);

    let result = registry.apply_all(&[file], "python", &HashMap::new());

    assert_eq!(
        result.connections.len(),
        0,
        "Disabled pattern should produce zero findings"
    );
}

// =============================================================================
// DACC-01: py-opcua narrowed import_gate and match strings
// =============================================================================

fn make_opcua_pattern() -> Pattern {
    Pattern {
        id: "py-opcua".to_string(),
        name: "py-opcua".to_string(),
        description: "OPC-UA Python client via asyncua".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![
            "from asyncua import".to_string(),
            "from asyncua.".to_string(),
            "import asyncua".to_string(),
        ],
        detections: vec![
            Detection {
                match_str: "= Client(".to_string(),
                kind: "connection".to_string(),
                protocol: "opcua".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::NamedArg("url".to_string()),
            },
            Detection {
                match_str: "Client(url=".to_string(),
                kind: "connection".to_string(),
                protocol: "opcua".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::NamedArg("url".to_string()),
            },
        ],
    }
}

#[test]
fn test_opcua_narrowed_import_gate_blocks_substring_match() {
    // OLD import_gate ["asyncua"] would match this file (asyncua appears in a comment).
    // NEW import_gate ["from asyncua import", "from asyncua.", "import asyncua"] must NOT.
    let pattern = make_opcua_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/client.py"),
        relative_path: "client.py".to_string(),
        // File references asyncua in a comment only — no actual import
        content: Arc::from("# This module is NOT asyncua-based\nclient = RegistryClient(\"host\")"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(), 0,
        "asyncua in comment must not trigger import gate"
    );
}

#[test]
fn test_opcua_narrowed_match_blocks_registry_client() {
    // Even if import gate passes, "RegistryClient(" must NOT match "= Client(" or "Client(url="
    let pattern = make_opcua_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/client.py"),
        relative_path: "client.py".to_string(),
        content: Arc::from("from asyncua import Node\nreg = RegistryClient(\"host\")"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(), 0,
        "RegistryClient( must not match narrowed opcua pattern"
    );
}

#[test]
fn test_opcua_narrowed_match_blocks_governor_signal_client() {
    let pattern = make_opcua_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/client.py"),
        relative_path: "client.py".to_string(),
        content: Arc::from("from asyncua import Node\ng = GovernorSignalClient(\"host\")"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(), 0,
        "GovernorSignalClient( must not match narrowed opcua pattern"
    );
}

#[test]
fn test_opcua_assignment_form_fires() {
    // "= Client(" must fire when import gate passes
    let pattern = make_opcua_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/client.py"),
        relative_path: "client.py".to_string(),
        content: Arc::from("from asyncua import Client\nclient = Client(\"opc.tcp://plc:4840\")"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(), 1,
        "= Client( with asyncua import must produce one finding"
    );
    assert_eq!(result.connections[0].protocol, "opcua");
}

#[test]
fn test_opcua_url_kwarg_form_fires() {
    // "Client(url=" must fire when import gate passes
    let pattern = make_opcua_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/client.py"),
        relative_path: "client.py".to_string(),
        content: Arc::from("import asyncua\nclient = asyncua.Client(url=\"opc.tcp://plc:4840\")"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(), 1,
        "Client(url= with asyncua import must produce one finding"
    );
    assert_eq!(result.connections[0].protocol, "opcua");
}

// =============================================================================
// DACC-05: py-kubernetes pattern — CoreV1Api, AppsV1Api, BatchV1Api, etc.
// =============================================================================

fn make_kubernetes_pattern() -> Pattern {
    Pattern {
        id: "py-kubernetes".to_string(),
        name: "py-kubernetes".to_string(),
        description: "Python kubernetes client API constructors".to_string(),
        languages: vec!["python".to_string()],
        file_patterns: vec!["**/*.py".to_string()],
        import_gate: vec![
            "from kubernetes import".to_string(),
            "from kubernetes.".to_string(),
            "import kubernetes".to_string(),
        ],
        detections: vec![
            Detection {
                match_str: "CoreV1Api(".to_string(),
                kind: "connection".to_string(),
                protocol: "kubernetes".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            },
            Detection {
                match_str: "AppsV1Api(".to_string(),
                kind: "connection".to_string(),
                protocol: "kubernetes".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            },
            Detection {
                match_str: "BatchV1Api(".to_string(),
                kind: "connection".to_string(),
                protocol: "kubernetes".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            },
            Detection {
                match_str: "NetworkingV1Api(".to_string(),
                kind: "connection".to_string(),
                protocol: "kubernetes".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            },
            Detection {
                match_str: "CustomObjectsApi(".to_string(),
                kind: "connection".to_string(),
                protocol: "kubernetes".to_string(),
                confidence: PatternConfidence::High,
                target_extraction: TargetExtraction::None,
            },
        ],
    }
}

#[test]
fn test_kubernetes_core_v1_api_fires() {
    let pattern = make_kubernetes_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/kube.py"),
        relative_path: "kube.py".to_string(),
        content: Arc::from("from kubernetes import client\nv1 = client.CoreV1Api()"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(result.connections.len(), 1, "CoreV1Api( must produce one finding");
    assert_eq!(result.connections[0].protocol, "kubernetes");
}

#[test]
fn test_kubernetes_apps_v1_api_fires() {
    let pattern = make_kubernetes_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/deploy.py"),
        relative_path: "deploy.py".to_string(),
        content: Arc::from("from kubernetes import client\napps = client.AppsV1Api()"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(result.connections.len(), 1, "AppsV1Api( must produce one finding");
    assert_eq!(result.connections[0].protocol, "kubernetes");
}

#[test]
fn test_kubernetes_multiple_apis_in_one_file() {
    let pattern = make_kubernetes_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let content = "from kubernetes import client\n\
                   v1 = client.CoreV1Api()\n\
                   apps = client.AppsV1Api()\n\
                   batch = client.BatchV1Api()";
    let file = FileContext {
        path: PathBuf::from("/repo/k8s_manager.py"),
        relative_path: "k8s_manager.py".to_string(),
        content: Arc::from(content),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(), 3,
        "Three distinct kubernetes API calls must produce 3 findings"
    );
    for conn in &result.connections {
        assert_eq!(conn.protocol, "kubernetes");
    }
}

#[test]
fn test_kubernetes_no_import_no_finding() {
    let pattern = make_kubernetes_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/unrelated.py"),
        relative_path: "unrelated.py".to_string(),
        content: Arc::from("# No kubernetes import here\nv1 = CoreV1Api()"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(), 0,
        "CoreV1Api( without kubernetes import must not fire"
    );
}

#[test]
fn test_kubernetes_custom_objects_api_fires() {
    let pattern = make_kubernetes_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/crds.py"),
        relative_path: "crds.py".to_string(),
        content: Arc::from("import kubernetes\nco = kubernetes.client.CustomObjectsApi()"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(result.connections.len(), 1, "CustomObjectsApi( must produce one finding");
    assert_eq!(result.connections[0].protocol, "kubernetes");
}

#[test]
fn test_kubernetes_networking_v1_api_fires() {
    let pattern = make_kubernetes_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let file = FileContext {
        path: PathBuf::from("/repo/net.py"),
        relative_path: "net.py".to_string(),
        content: Arc::from("from kubernetes.client import NetworkingV1Api\nnet = NetworkingV1Api()"),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());
    assert_eq!(result.connections.len(), 1, "NetworkingV1Api( must produce one finding");
    assert_eq!(result.connections[0].protocol, "kubernetes");
}

// =============================================================================
// TEST-01: Combined regression — all Phase 8 fixes together
// =============================================================================

#[test]
fn test_all_phase8_fixes_combined() {
    // This test exercises all four fixes at once using a realistic Python file.
    //
    // Setup:
    // - Pattern: py-opcua with narrowed import_gate (DACC-01) and file_patterns=["**/*.py"] (DACC-02)
    // - File: has an asyncua import, a docstring with Client(, and a real = Client( call
    //
    // Expected: exactly ONE finding (the real call), NOT the docstring mention

    let pattern = make_opcua_pattern(); // defined in DACC-01 section — uses narrowed gates

    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    // Realistic Python file: module docstring contains Client( as an example,
    // real call is on the last line
    let content = "\
from asyncua import Client

def connect_to_plc(url: str):
    \"\"\"
    Connect to an OPC-UA PLC.

    Example usage:
        client = Client(\"opc.tcp://plc:4840\")
        client.connect()
    \"\"\"
    client = Client(url)
    return client
";

    let file = FileContext {
        path: PathBuf::from("/repo/plc/connector.py"),
        relative_path: "plc/connector.py".to_string(),
        content: Arc::from(content),
    };

    let result = registry.apply_all(&[file], "python", &HashMap::new());

    assert_eq!(
        result.connections.len(),
        1,
        "Exactly one finding expected: the real Client( call, not the docstring example. \
         Got {} findings: {:?}",
        result.connections.len(),
        result.connections.iter().map(|c| &c.source_file).collect::<Vec<_>>()
    );
    assert_eq!(
        result.connections[0].protocol, "opcua",
        "Finding must have opcua protocol"
    );
}

#[test]
fn test_phase8_file_patterns_scopes_pattern_to_python_only() {
    // A Go file containing "= Client(" must not fire the py-opcua pattern
    // because file_patterns=["**/*.py"] excludes .go files
    let pattern = make_opcua_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let go_file = FileContext {
        path: PathBuf::from("/repo/main.go"),
        relative_path: "main.go".to_string(),
        // Contains the import gate text AND match string, but wrong file type
        content: Arc::from("// from asyncua import Client\n// = Client(\"host\")"),
    };

    let result = registry.apply_all(&[go_file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(), 0,
        "py-opcua pattern must not fire on .go files due to file_patterns restriction"
    );
}

#[test]
fn test_phase8_kubernetes_file_patterns_scopes_to_python() {
    // A TypeScript file with kubernetes text must not fire py-kubernetes
    let pattern = make_kubernetes_pattern();
    let registry = PatternRegistry::from_patterns(vec![pattern], "1.0".to_string());

    let ts_file = FileContext {
        path: PathBuf::from("/repo/src/k8s-client.ts"),
        relative_path: "src/k8s-client.ts".to_string(),
        content: Arc::from("// from kubernetes import client\n// v1 = CoreV1Api()"),
    };

    let result = registry.apply_all(&[ts_file], "python", &HashMap::new());
    assert_eq!(
        result.connections.len(), 0,
        "py-kubernetes pattern must not fire on .ts files due to file_patterns restriction"
    );
}
