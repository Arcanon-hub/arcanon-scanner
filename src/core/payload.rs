//! ScanPayloadV1 assembly — convert MergedResult into the final JSON payload format.
//! Produces JSON matching the exact shape expected by Arcanon Hub.

use crate::core::merger::MergedResult;
use crate::types::{Confidence, FieldInfo};
use serde::Serialize;
use std::collections::HashMap;

/// Top-level ScanPayloadV1 matching hub's expected format.
#[derive(Debug, Clone, Serialize)]
pub struct ScanPayloadV1 {
    pub version: &'static str,
    pub metadata: ScanMetadata,
    pub findings: ScanFindings,
}

/// Metadata about the scan.
#[derive(Debug, Clone, Serialize)]
pub struct ScanMetadata {
    pub tool: &'static str,
    pub tool_version: &'static str,
    pub scan_mode: &'static str,
    pub repo_url: Option<String>,
    pub repo_name: String,
    pub branch: String,
    pub commit_sha: String,
    pub started_at: String,
    pub completed_at: String,
    pub files_scanned: usize,
    pub project_slug: String,
    pub pattern_version: String,
    pub pattern_source: String,
}

/// All findings from the scan.
#[derive(Debug, Clone, Serialize)]
pub struct ScanFindings {
    pub services: Vec<ServicePayload>,
    pub connections: Vec<ConnectionPayload>,
    pub schemas: Vec<SchemaPayload>,
    pub actors: Vec<()>,
}

/// Service in the payload (with nested endpoints as "exposes").
#[derive(Debug, Clone, Serialize)]
pub struct ServicePayload {
    pub name: String,
    pub root_path: String,
    pub language: String,
    #[serde(rename = "type")]
    pub service_type: String,
    pub boundary_entry: Option<String>,
    pub confidence: String, // "high", "medium", "low"
    pub exposes: Vec<EndpointPayload>,
}

/// Endpoint in the payload.
#[derive(Debug, Clone, Serialize)]
pub struct EndpointPayload {
    pub method: String,
    pub path: String,
    pub handler: Option<String>,
    pub kind: String,
}

/// Connection in the payload (uses "source" and "target", not "source_service"/"target_name").
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionPayload {
    pub source: String,
    pub target: String,
    pub protocol: String,
    pub method: Option<String>,
    pub path: Option<String>,
    pub source_file: String,
    pub confidence: String,
    pub extraction_method: String, // passed through from ConnectionInfo
    pub dependency: Option<String>, // passed through from ConnectionInfo
    pub evidence: Option<String>,
}

/// Schema in the payload.
#[derive(Debug, Clone, Serialize)]
pub struct SchemaPayload {
    pub name: String,
    pub role: String,
    pub file: Option<String>,
    pub connection_ref: Option<String>,
    pub fields: Vec<FieldPayload>,
}

/// Field in a schema payload.
#[derive(Debug, Clone, Serialize)]
pub struct FieldPayload {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub required: bool,
}

/// Convert a Confidence enum to lowercase string for JSON serialization.
fn confidence_str(c: &Confidence) -> String {
    match c {
        Confidence::High => "high".to_string(),
        Confidence::Medium => "medium".to_string(),
        Confidence::Low => "low".to_string(),
    }
}

/// Assemble a ScanPayloadV1 from a MergedResult and context.
///
/// Maps:
/// - MergedResult.services → ServicePayload with nested endpoints as "exposes"
/// - MergedResult.connections → ConnectionPayload (using "source"/"target" field names)
/// - MergedResult.schemas → SchemaPayload
///
/// Parameters:
/// - `merged`: The deduplicated and resolved MergedResult from merger+resolver
/// - `repo_url`: Optional git repository URL
/// - `repo_name`: Repository name (e.g., "order-service")
/// - `branch`: Git branch name
/// - `commit_sha`: Full or short commit SHA
/// - `project_slug`: Project identifier for hub (e.g., "acme/order-system")
/// - `started_at`: RFC3339 timestamp when scan started
/// - `completed_at`: RFC3339 timestamp when scan completed
/// - `files_scanned`: Count of files processed during scan
/// - `pattern_version`: Version string from PatternFile, or "" when no patterns
/// - `pattern_source`: "remote", "cache", or "none" matching PatternSource
#[allow(clippy::too_many_arguments)]
pub fn assemble(
    merged: MergedResult,
    repo_url: Option<String>,
    repo_name: String,
    branch: String,
    commit_sha: String,
    project_slug: String,
    started_at: String,
    completed_at: String,
    files_scanned: usize,
    pattern_version: String,
    pattern_source: String,
) -> ScanPayloadV1 {
    // Build endpoint lookup: service_name → endpoints
    let mut endpoints_by_service: HashMap<String, Vec<EndpointPayload>> = HashMap::new();
    for ep in merged.endpoints {
        let payload_ep = EndpointPayload {
            method: ep.method,
            path: ep.path,
            handler: ep.handler,
            kind: ep.kind,
        };
        endpoints_by_service
            .entry(ep.service_name)
            .or_default()
            .push(payload_ep);
    }

    // Convert services to ServicePayload, including their endpoints as "exposes"
    let services: Vec<ServicePayload> = merged
        .services
        .into_iter()
        .map(|svc| ServicePayload {
            name: svc.name.clone(),
            root_path: svc.root_path.clone(),
            language: svc.language,
            service_type: svc.service_type,
            boundary_entry: svc.boundary_entry,
            confidence: confidence_str(&svc.confidence),
            exposes: endpoints_by_service.remove(&svc.name).unwrap_or_default(),
        })
        .collect();

    // Convert connections to ConnectionPayload (source/target field names)
    let connections: Vec<ConnectionPayload> = merged
        .connections
        .into_iter()
        .map(|conn| ConnectionPayload {
            source: conn.source_service,
            target: conn.target_name,
            protocol: conn.protocol,
            method: conn.method,
            path: conn.path,
            source_file: conn.source_file,
            confidence: confidence_str(&conn.confidence),
            extraction_method: conn.extraction_method,
            dependency: conn.dependency,
            evidence: conn.evidence,
        })
        .collect();

    // Convert schemas to SchemaPayload
    let schemas: Vec<SchemaPayload> = merged
        .schemas
        .into_iter()
        .map(|schema| SchemaPayload {
            name: schema.name,
            role: schema.role,
            file: schema.file,
            connection_ref: schema.connection_ref,
            fields: schema
                .fields
                .into_iter()
                .map(|f: FieldInfo| FieldPayload {
                    name: f.name,
                    field_type: f.field_type,
                    required: f.required,
                })
                .collect(),
        })
        .collect();

    ScanPayloadV1 {
        version: "1.0",
        metadata: ScanMetadata {
            tool: "cli",
            tool_version: "0.1.0",
            scan_mode: "full",
            repo_url,
            repo_name,
            branch,
            commit_sha,
            started_at,
            completed_at,
            files_scanned,
            project_slug,
            pattern_version,
            pattern_source,
        },
        findings: ScanFindings {
            services,
            connections,
            schemas,
            actors: vec![], // Always empty in v1
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    #[test]
    fn test_confidence_str() {
        assert_eq!(confidence_str(&Confidence::High), "high");
        assert_eq!(confidence_str(&Confidence::Medium), "medium");
        assert_eq!(confidence_str(&Confidence::Low), "low");
    }

    #[test]
    fn test_assemble_single_service_with_endpoints() {
        let merged = MergedResult {
            services: vec![ServiceInfo {
                name: "order-service".to_string(),
                root_path: ".".to_string(),
                language: "typescript".to_string(),
                service_type: "service".to_string(),
                boundary_entry: Some("src/main.ts".to_string()),
                confidence: Confidence::High,
                extraction_method: "compose".to_string(),
                ..Default::default()
            }],
            endpoints: vec![
                EndpointInfo {
                    service_name: "order-service".to_string(),
                    method: "POST".to_string(),
                    path: "/api/v1/orders".to_string(),
                    handler: Some("createOrder".to_string()),
                    kind: "rest".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "ast:typescript".to_string(),
                    ..Default::default()
                },
                EndpointInfo {
                    service_name: "order-service".to_string(),
                    method: "GET".to_string(),
                    path: "/api/v1/orders/{id}".to_string(),
                    handler: Some("getOrder".to_string()),
                    kind: "rest".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "ast:typescript".to_string(),
                    ..Default::default()
                },
            ],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let payload = assemble(
            merged,
            Some("https://github.com/acme/order-service".to_string()),
            "order-service".to_string(),
            "main".to_string(),
            "abc1234".to_string(),
            "acme/order-system".to_string(),
            "2026-04-04T10:00:00Z".to_string(),
            "2026-04-04T10:00:03Z".to_string(),
            247,
            "".to_string(),
            "none".to_string(),
        );

        // Verify top-level structure
        assert_eq!(payload.version, "1.0");
        assert_eq!(payload.metadata.tool, "cli");
        assert_eq!(payload.metadata.tool_version, "0.1.0");
        assert_eq!(payload.metadata.scan_mode, "full");
        assert_eq!(payload.metadata.repo_name, "order-service");
        assert_eq!(payload.metadata.files_scanned, 247);

        // Verify service structure with nested endpoints
        assert_eq!(payload.findings.services.len(), 1);
        assert_eq!(payload.findings.services[0].name, "order-service");
        assert_eq!(payload.findings.services[0].exposes.len(), 2);
        assert_eq!(payload.findings.services[0].confidence, "high");

        // Verify endpoints are nested
        let endpoint1 = &payload.findings.services[0].exposes[0];
        assert_eq!(endpoint1.method, "POST");
        assert_eq!(endpoint1.path, "/api/v1/orders");

        // Verify actors is empty
        assert_eq!(payload.findings.actors.len(), 0);
    }

    #[test]
    fn test_assemble_connection_field_names() {
        let merged = MergedResult {
            services: vec![],
            endpoints: vec![],
            connections: vec![ConnectionInfo {
                source_service: "order-service".to_string(),
                target_name: "payment-service".to_string(),
                protocol: "rest".to_string(),
                method: Some("POST".to_string()),
                path: Some("/api/v1/payments/charge".to_string()),
                source_file: "src/services/payment.ts:42".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
                dependency: None,
                evidence: Some("axios.post(...)".to_string()),
                ..Default::default()
            }],
            schemas: vec![],
            actors: vec![],
        };

        let payload = assemble(
            merged,
            None,
            "test-repo".to_string(),
            "main".to_string(),
            "123".to_string(),
            "acme/test".to_string(),
            "2026-04-04T10:00:00Z".to_string(),
            "2026-04-04T10:00:01Z".to_string(),
            10,
            "".to_string(),
            "none".to_string(),
        );

        // Verify connection uses "source" and "target" field names
        assert_eq!(payload.findings.connections.len(), 1);
        let conn = &payload.findings.connections[0];
        assert_eq!(conn.source, "order-service");
        assert_eq!(conn.target, "payment-service");
        assert_eq!(conn.protocol, "rest");
    }

    #[test]
    fn test_assemble_schema_payload() {
        let merged = MergedResult {
            services: vec![],
            endpoints: vec![],
            connections: vec![],
            schemas: vec![SchemaInfo {
                name: "CreateOrderRequest".to_string(),
                role: "request".to_string(),
                file: Some("src/models/order.ts".to_string()),
                connection_ref: None,
                fields: vec![
                    FieldInfo {
                        name: "product_id".to_string(),
                        field_type: "string".to_string(),
                        required: true,
                    },
                    FieldInfo {
                        name: "quantity".to_string(),
                        field_type: "integer".to_string(),
                        required: false,
                    },
                ],
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
                ..Default::default()
            }],
            actors: vec![],
        };

        let payload = assemble(
            merged,
            None,
            "test".to_string(),
            "main".to_string(),
            "123".to_string(),
            "acme/test".to_string(),
            "2026-04-04T10:00:00Z".to_string(),
            "2026-04-04T10:00:01Z".to_string(),
            10,
            "".to_string(),
            "none".to_string(),
        );

        assert_eq!(payload.findings.schemas.len(), 1);
        let schema = &payload.findings.schemas[0];
        assert_eq!(schema.name, "CreateOrderRequest");
        assert_eq!(schema.role, "request");
        assert_eq!(schema.fields.len(), 2);
        assert_eq!(schema.fields[0].name, "product_id");
        assert_eq!(schema.fields[0].field_type, "string");
        assert_eq!(schema.fields[0].required, true);
        assert_eq!(schema.fields[1].required, false);
    }

    #[test]
    fn test_assemble_serializes_to_valid_json() {
        let merged = MergedResult {
            services: vec![ServiceInfo {
                name: "test-service".to_string(),
                root_path: ".".to_string(),
                language: "python".to_string(),
                service_type: "service".to_string(),
                boundary_entry: None,
                confidence: Confidence::Medium,
                extraction_method: "ast:python".to_string(),
                ..Default::default()
            }],
            endpoints: vec![],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let payload = assemble(
            merged,
            None,
            "test".to_string(),
            "main".to_string(),
            "123".to_string(),
            "test/project".to_string(),
            "2026-04-04T10:00:00Z".to_string(),
            "2026-04-04T10:00:01Z".to_string(),
            5,
            "".to_string(),
            "none".to_string(),
        );

        // Should serialize to valid JSON
        let json_str = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json_str.contains("\"version\":\"1.0\""));
        assert!(json_str.contains("\"tool\":\"cli\""));
        assert!(json_str.contains("\"actors\":[]"));

        // Should deserialize back correctly
        let deserialized: serde_json::Value =
            serde_json::from_str(&json_str).expect("deserialization failed");
        assert_eq!(deserialized["version"], "1.0");
        assert_eq!(deserialized["metadata"]["tool"], "cli");
        assert!(deserialized["findings"]["actors"].is_array());
    }

    #[test]
    fn test_assemble_service_type_serialization() {
        let merged = MergedResult {
            services: vec![ServiceInfo {
                name: "db".to_string(),
                root_path: "db".to_string(),
                language: "".to_string(),
                service_type: "database".to_string(),
                boundary_entry: None,
                confidence: Confidence::High,
                extraction_method: "test".to_string(),
                ..Default::default()
            }],
            endpoints: vec![],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let payload = assemble(
            merged,
            None,
            "test".to_string(),
            "main".to_string(),
            "123".to_string(),
            "test/project".to_string(),
            "2026-04-04T10:00:00Z".to_string(),
            "2026-04-04T10:00:01Z".to_string(),
            5,
            "".to_string(),
            "none".to_string(),
        );

        // Verify that service_type is serialized as "type" in JSON
        let json_str = serde_json::to_string(&payload).expect("serialization failed");
        assert!(json_str.contains("\"type\":\"database\""));
        assert!(!json_str.contains("\"service_type\""));
    }

    #[test]
    fn test_assemble_multiple_services_separate_endpoints() {
        let merged = MergedResult {
            services: vec![
                ServiceInfo {
                    name: "api".to_string(),
                    root_path: "api".to_string(),
                    language: "typescript".to_string(),
                    service_type: "service".to_string(),
                    boundary_entry: None,
                    confidence: Confidence::High,
                    extraction_method: "test".to_string(),
                    ..Default::default()
                },
                ServiceInfo {
                    name: "worker".to_string(),
                    root_path: "worker".to_string(),
                    language: "python".to_string(),
                    service_type: "service".to_string(),
                    boundary_entry: None,
                    confidence: Confidence::High,
                    extraction_method: "test".to_string(),
                    ..Default::default()
                }
            ],
            endpoints: vec![
                EndpointInfo {
                    service_name: "api".to_string(),
                    method: "GET".to_string(),
                    path: "/health".to_string(),
                    handler: None,
                    kind: "rest".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "ast:typescript".to_string(),
                    ..Default::default()
                },
                EndpointInfo {
                    service_name: "worker".to_string(),
                    method: "POST".to_string(),
                    path: "/process".to_string(),
                    handler: None,
                    kind: "rest".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "ast:python".to_string(),
                    ..Default::default()
                },
            ],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let payload = assemble(
            merged,
            None,
            "test".to_string(),
            "main".to_string(),
            "123".to_string(),
            "test/project".to_string(),
            "2026-04-04T10:00:00Z".to_string(),
            "2026-04-04T10:00:01Z".to_string(),
            20,
            "".to_string(),
            "none".to_string(),
        );

        assert_eq!(payload.findings.services.len(), 2);

        // Find api service and verify its endpoints
        let api = payload
            .findings
            .services
            .iter()
            .find(|s| s.name == "api")
            .expect("api service not found");
        assert_eq!(api.exposes.len(), 1);
        assert_eq!(api.exposes[0].path, "/health");

        // Find worker service and verify its endpoints
        let worker = payload
            .findings
            .services
            .iter()
            .find(|s| s.name == "worker")
            .expect("worker service not found");
        assert_eq!(worker.exposes.len(), 1);
        assert_eq!(worker.exposes[0].path, "/process");
    }

    #[test]
    fn test_connection_payload_includes_extraction_method_and_dependency() {
        let merged = MergedResult {
            services: vec![],
            endpoints: vec![],
            connections: vec![ConnectionInfo {
                source_service: "svc".to_string(),
                target_name: "cache".to_string(),
                protocol: "redis".to_string(),
                method: None,
                path: None,
                source_file: "src/cache.py:10".to_string(),
                confidence: Confidence::High,
                extraction_method: "pattern:py-redis".to_string(),
                dependency: Some("py-redis".to_string()),
                evidence: None,
                ..Default::default()
            }],
            schemas: vec![],
            actors: vec![],
        };
        let payload = assemble(
            merged,
            None,
            "repo".to_string(),
            "main".to_string(),
            "abc".to_string(),
            "org/repo".to_string(),
            "2026-04-07T00:00:00Z".to_string(),
            "2026-04-07T00:00:01Z".to_string(),
            1,
            "".to_string(),
            "none".to_string(),
        );
        let conn = &payload.findings.connections[0];
        assert_eq!(conn.extraction_method, "pattern:py-redis");
        assert_eq!(conn.dependency, Some("py-redis".to_string()));
    }

    #[test]
    fn test_connection_payload_null_dependency_serializes_as_null() {
        let merged = MergedResult {
            services: vec![],
            endpoints: vec![],
            connections: vec![ConnectionInfo {
                source_service: "svc".to_string(),
                target_name: "api".to_string(),
                protocol: "rest".to_string(),
                method: None,
                path: None,
                source_file: "src/client.ts:5".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
                dependency: None,
                evidence: None,
                ..Default::default()
            }],
            schemas: vec![],
            actors: vec![],
        };
        let payload = assemble(
            merged,
            None,
            "repo".to_string(),
            "main".to_string(),
            "abc".to_string(),
            "org/repo".to_string(),
            "2026-04-07T00:00:00Z".to_string(),
            "2026-04-07T00:00:01Z".to_string(),
            1,
            "".to_string(),
            "none".to_string(),
        );
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"extraction_method\":\"ast:typescript\""));
        assert!(json.contains("\"dependency\":null"));
    }
}
