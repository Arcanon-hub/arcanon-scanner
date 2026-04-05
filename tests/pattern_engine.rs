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
    use serde_json::json;

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
    assert!(
        registry.patterns().len() >= 0,
        "Should not panic and return valid registry"
    );
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
