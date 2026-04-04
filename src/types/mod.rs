//! Shared types used across all plugins and the core engine.
//! Defined in Phase 1 Plan 02 to establish the type contract.
//! Every field matches docs/architecture.md section 5 exactly.

/// Confidence level for a finding (service, endpoint, connection, schema).
#[derive(Debug, Clone, PartialEq)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// A field within a schema (request, response, or event).
#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub field_type: String,
    pub required: bool,
}

/// Actor information — empty struct in Phase 1, full definition deferred.
#[derive(Debug, Clone)]
pub struct ActorInfo {}

/// Service detected in the codebase.
#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub name: String,
    pub root_path: String,
    pub language: String,
    pub service_type: String, // "service", "frontend", "database", "broker", "external"
    pub boundary_entry: Option<String>,
    pub confidence: Confidence,
    pub extraction_method: String,
}

/// Endpoint (HTTP route, gRPC service, GraphQL query/mutation, etc.)
#[derive(Debug, Clone)]
pub struct EndpointInfo {
    pub service_name: String,
    pub method: String,
    pub path: String,
    pub handler: Option<String>,
    pub kind: String, // "rest", "grpc", "graphql", "websocket"
    pub confidence: Confidence,
    pub extraction_method: String,
}

/// Connection from one service to another (HTTP call, DB access, message queue, etc.)
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub source_service: String,
    pub target_name: String,
    pub protocol: String, // free string — no enum; supports "rest", "grpc", "kafka", "postgresql", etc.
    pub method: Option<String>,
    pub path: Option<String>,
    pub source_file: String, // "file:line" format
    pub confidence: Confidence,
    pub extraction_method: String,
    pub evidence: Option<String>,
}

/// Schema definition (request body, response, event payload, etc.)
#[derive(Debug, Clone)]
pub struct SchemaInfo {
    pub name: String,
    pub role: String, // "request", "response", "event"
    pub file: Option<String>,
    pub connection_ref: Option<String>,
    pub fields: Vec<FieldInfo>,
    pub confidence: Confidence,
    pub extraction_method: String,
}

/// Complete extraction result from a single plugin.
#[derive(Debug, Clone, Default)]
pub struct ExtractionResult {
    pub services: Vec<ServiceInfo>,
    pub endpoints: Vec<EndpointInfo>,
    pub connections: Vec<ConnectionInfo>,
    pub schemas: Vec<SchemaInfo>,
    pub actors: Vec<ActorInfo>,
}
