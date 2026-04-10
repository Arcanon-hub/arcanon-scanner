use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{
    Confidence, ConnectionInfo, EndpointInfo, ExtractionResult, FieldInfo, SchemaInfo,
};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::url_util::parse_url_value;

/// Type alias for OpenAPI/Swagger parse result
type ParseResult = Result<
    (
        Option<crate::types::ServiceInfo>,
        Vec<EndpointInfo>,
        Vec<SchemaInfo>,
        Vec<ConnectionInfo>,
    ),
    String,
>;

/// OpenAPI 3.0 and Swagger 2.0 specification parser.
pub struct OpenApiPlugin;

impl LanguagePlugin for OpenApiPlugin {
    fn name(&self) -> &str {
        "openapi"
    }

    fn file_patterns(&self) -> &[&str] {
        &[
            "**/openapi.json",
            "**/openapi.yaml",
            "**/openapi.yml",
            "**/swagger.json",
            "**/swagger.yaml",
            "**/swagger.yml",
            "**/*.openapi.json",
            "**/*.openapi.yaml",
            "**/*.openapi.yml",
            "**/*.swagger.json",
            "**/*.swagger.yaml",
            "**/*.swagger.yml",
        ]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        for file in &ctx.files {
            // Detect format and parse accordingly
            if file.content.contains("\"openapi\": \"3.") || file.content.contains("openapi: \"3.")
            {
                // OpenAPI 3.0
                match parse_openapi_3(&file.content, &file.relative_path) {
                    Ok((service, endpoints, schemas, connections)) => {
                        if let Some(svc) = service {
                            result.services.push(svc);
                        }
                        result.endpoints.extend(endpoints);
                        result.schemas.extend(schemas);
                        result.connections.extend(connections);
                    }
                    Err(e) => {
                        warn!("openapi: failed to parse {}: {}", file.relative_path, e);
                    }
                }
            } else if file.content.contains("\"swagger\": \"2.")
                || file.content.contains("swagger: \"2.")
            {
                // Swagger 2.0
                match parse_swagger_2(&file.content, &file.relative_path) {
                    Ok((service, endpoints, schemas, connections)) => {
                        if let Some(svc) = service {
                            result.services.push(svc);
                        }
                        result.endpoints.extend(endpoints);
                        result.schemas.extend(schemas);
                        result.connections.extend(connections);
                    }
                    Err(e) => {
                        warn!("openapi: failed to parse {}: {}", file.relative_path, e);
                    }
                }
            }
        }

        result
    }
}

/// Parse OpenAPI 3.0 specification
fn parse_openapi_3(content: &str, relative_path: &str) -> ParseResult {
    let mut endpoints = Vec::new();
    let mut schemas = Vec::new();

    // Try JSON first, then YAML
    let spec: openapiv3::OpenAPI = if content.trim().starts_with('{') {
        serde_json::from_str(content).map_err(|e| format!("JSON parse error: {}", e))?
    } else {
        serde_yaml_bw::from_str(content).map_err(|e| format!("YAML parse error: {}", e))?
    };

    // Get service name from title (always present in OpenAPI 3.0)
    let service_name = spec.info.title.clone();

    // Create a ServiceInfo for the API
    let service = crate::types::ServiceInfo {
        name: service_name.clone(),
        root_path: String::new(),
        language: String::new(),
        service_type: "service".to_string(),
        boundary_entry: Some(relative_path.to_string()),
        confidence: Confidence::High,
        extraction_method: "spec:openapi".to_string(),
        ..Default::default()
    };

    // Extract connections from servers
    let mut connections = Vec::new();
    for server in &spec.servers {
        if let Some((protocol, hostname)) = parse_url_value(&server.url) {
            connections.push(ConnectionInfo {
                source_service: service_name.clone(),
                target_name: hostname,
                protocol,
                method: None,
                path: None,
                source_file: format!("{}:0", relative_path),
                confidence: Confidence::Medium,
                extraction_method: "spec:openapi".to_string(),
                dependency: None,
                evidence: Some(format!("servers[].url = {}", server.url)),
                ..Default::default()
            });
        }
    }

    // Extract endpoints from paths
    let paths_map = spec.paths.paths;
    for (path_str, path_item_ref) in paths_map.iter() {
        // Skip references that can't be resolved
        if let Some(path_item) = path_item_ref.as_item() {
            // Extract each HTTP method
            for (method, operation) in [
                ("GET", path_item.get.as_ref()),
                ("POST", path_item.post.as_ref()),
                ("PUT", path_item.put.as_ref()),
                ("DELETE", path_item.delete.as_ref()),
                ("PATCH", path_item.patch.as_ref()),
                ("OPTIONS", path_item.options.as_ref()),
                ("HEAD", path_item.head.as_ref()),
                ("TRACE", path_item.trace.as_ref()),
            ] {
                if let Some(op) = operation {
                    endpoints.push(EndpointInfo {
                        service_name: service_name.clone(),
                        method: method.to_string(),
                        path: path_str.clone(),
                        handler: op.operation_id.clone(),
                        kind: "rest".to_string(),
                        confidence: Confidence::High,
                        extraction_method: "spec:openapi".to_string(),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Extract schemas from components
    if let Some(components) = spec.components {
        let schemas_map = components.schemas;
        for (schema_name, schema_ref) in schemas_map.iter() {
            if let Some(schema) = schema_ref.as_item() {
                let fields = extract_fields(schema);
                schemas.push(SchemaInfo {
                    name: schema_name.clone(),
                    role: "type".to_string(),
                    file: Some(relative_path.to_string()),
                    connection_ref: None,
                    fields,
                    confidence: Confidence::High,
                    extraction_method: "spec:openapi".to_string(),
                    ..Default::default()
                });
            }
        }
    }

    Ok((Some(service), endpoints, schemas, connections))
}

/// Extract fields from an OpenAPI Schema
fn extract_fields(schema: &openapiv3::Schema) -> Vec<FieldInfo> {
    let mut fields = Vec::new();

    if let openapiv3::SchemaKind::Type(openapiv3::Type::Object(obj)) = &schema.schema_kind {
        let properties = &obj.properties;
        let required = &obj.required;

        for (prop_name, prop_schema_ref) in properties.iter() {
            let field_type = if let Some(prop_schema) = prop_schema_ref.as_item() {
                format_type(prop_schema)
            } else {
                "unknown".to_string()
            };

            let is_required = required.iter().any(|r| r == prop_name);

            fields.push(FieldInfo {
                name: prop_name.clone(),
                field_type,
                required: is_required,
            });
        }
    }

    fields
}

/// Format OpenAPI type to string
fn format_type(schema: &openapiv3::Schema) -> String {
    match &schema.schema_kind {
        openapiv3::SchemaKind::Type(openapiv3::Type::String(_)) => "string".to_string(),
        openapiv3::SchemaKind::Type(openapiv3::Type::Number(_)) => "number".to_string(),
        openapiv3::SchemaKind::Type(openapiv3::Type::Integer(_)) => "integer".to_string(),
        openapiv3::SchemaKind::Type(openapiv3::Type::Boolean(_)) => "boolean".to_string(),
        openapiv3::SchemaKind::Type(openapiv3::Type::Array(arr)) => {
            let item_type = if let Some(item_ref) = &arr.items {
                if let Some(item_schema) = item_ref.as_item() {
                    format_type(item_schema)
                } else {
                    "unknown".to_string()
                }
            } else {
                "unknown".to_string()
            };
            format!("array[{}]", item_type)
        }
        openapiv3::SchemaKind::Type(openapiv3::Type::Object(_)) => "object".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Swagger 2.0 specification (custom structs)
#[derive(Debug, Deserialize, Serialize, Default)]
struct SwaggerSpec {
    swagger: Option<String>,
    info: SwaggerInfo,
    host: Option<String>,
    base_path: Option<String>,
    paths: Option<std::collections::HashMap<String, SwaggerPathItem>>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct SwaggerInfo {
    title: Option<String>,
    version: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct SwaggerPathItem {
    get: Option<SwaggerOperation>,
    post: Option<SwaggerOperation>,
    put: Option<SwaggerOperation>,
    delete: Option<SwaggerOperation>,
    patch: Option<SwaggerOperation>,
    options: Option<SwaggerOperation>,
    head: Option<SwaggerOperation>,
    trace: Option<SwaggerOperation>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct SwaggerOperation {
    #[serde(rename = "operationId")]
    operation_id: Option<String>,
}

/// Parse Swagger 2.0 specification
fn parse_swagger_2(content: &str, relative_path: &str) -> ParseResult {
    let mut endpoints = Vec::new();

    // Try JSON first, then YAML
    let spec: SwaggerSpec = if content.trim().starts_with('{') {
        serde_json::from_str(content).map_err(|e| format!("JSON parse error: {}", e))?
    } else {
        serde_yaml_bw::from_str(content).map_err(|e| format!("YAML parse error: {}", e))?
    };

    // Get service name
    let service_name = if let Some(title) = spec.info.title {
        title
    } else {
        std::path::Path::new(relative_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("api")
            .to_string()
    };

    // Create a ServiceInfo for the API
    let service = crate::types::ServiceInfo {
        name: service_name.clone(),
        root_path: String::new(),
        language: String::new(),
        service_type: "service".to_string(),
        boundary_entry: Some(relative_path.to_string()),
        confidence: Confidence::High,
        extraction_method: "spec:openapi".to_string(),
        ..Default::default()
    };

    // Extract connection from host field
    let mut connections = Vec::new();
    if let Some(host) = &spec.host {
        // Swagger 2.0 host is bare hostname[:port], not a full URL — prepend http:// for parsing
        let synthetic_url = format!("http://{}", host);
        if let Some((_protocol, hostname)) = parse_url_value(&synthetic_url) {
            connections.push(ConnectionInfo {
                source_service: service_name.clone(),
                target_name: hostname,
                protocol: "http".to_string(), // Swagger 2.0 host is scheme-agnostic; default http
                method: None,
                path: None,
                source_file: format!("{}:0", relative_path),
                confidence: Confidence::Medium,
                extraction_method: "spec:openapi".to_string(),
                dependency: None,
                evidence: Some(format!("host: {}", host)),
                ..Default::default()
            });
        }
    }

    // Extract endpoints from paths
    if let Some(paths) = spec.paths {
        for (path_str, path_item) in paths.iter() {
            for (method, operation) in [
                ("GET", path_item.get.as_ref()),
                ("POST", path_item.post.as_ref()),
                ("PUT", path_item.put.as_ref()),
                ("DELETE", path_item.delete.as_ref()),
                ("PATCH", path_item.patch.as_ref()),
                ("OPTIONS", path_item.options.as_ref()),
                ("HEAD", path_item.head.as_ref()),
                ("TRACE", path_item.trace.as_ref()),
            ] {
                if let Some(op) = operation {
                    endpoints.push(EndpointInfo {
                        service_name: service_name.clone(),
                        method: method.to_string(),
                        path: path_str.clone(),
                        handler: op.operation_id.clone(),
                        kind: "rest".to_string(),
                        confidence: Confidence::High,
                        extraction_method: "spec:openapi".to_string(),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Swagger 2.0 schemas are in top-level definitions, not components
    Ok((Some(service), endpoints, Vec::new(), connections))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oas3_yaml_endpoints() {
        let yaml = r#"
openapi: "3.0.3"
info:
  title: User Service
  version: "1.0.0"
paths:
  /api/v1/users:
    get:
      operationId: listUsers
      responses:
        "200":
          description: Success
    post:
      operationId: createUser
      responses:
        "201":
          description: Created
  /api/v1/users/{id}:
    get:
      operationId: getUser
      responses:
        "200":
          description: Success
"#;

        let (_service, endpoints, _schemas, _connections) =
            parse_openapi_3(yaml, "test.yaml").expect("parse failed");

        assert_eq!(endpoints.len(), 3);
        assert!(endpoints
            .iter()
            .any(|e| e.method == "GET" && e.path == "/api/v1/users"));
        assert!(endpoints
            .iter()
            .any(|e| e.method == "POST" && e.path == "/api/v1/users"));
        assert!(endpoints
            .iter()
            .any(|e| e.method == "GET" && e.path == "/api/v1/users/{id}"));

        for endpoint in &endpoints {
            assert_eq!(endpoint.kind, "rest");
            assert_eq!(endpoint.extraction_method, "spec:openapi");
            assert_eq!(endpoint.confidence, Confidence::High);
        }
    }

    #[test]
    fn test_oas3_schema_extraction() {
        let yaml = r#"
openapi: "3.0.3"
info:
  title: User Service
  version: "1.0.0"
paths: {}
components:
  schemas:
    CreateUserRequest:
      type: object
      required: [email, name]
      properties:
        email:
          type: string
        name:
          type: string
"#;

        let (_service, _endpoints, schemas, _connections) =
            parse_openapi_3(yaml, "test.yaml").expect("parse failed");

        assert_eq!(schemas.len(), 1);
        let schema = &schemas[0];
        assert_eq!(schema.name, "CreateUserRequest");
        assert_eq!(schema.role, "type");
        assert_eq!(schema.extraction_method, "spec:openapi");
        assert_eq!(schema.fields.len(), 2);

        let email_field = schema.fields.iter().find(|f| f.name == "email").unwrap();
        assert_eq!(email_field.field_type, "string");
        assert!(email_field.required);
    }

    #[test]
    fn test_oas3_invalid_yaml() {
        let invalid = "{ invalid yaml [[[";
        let result = parse_openapi_3(invalid, "test.yaml");
        assert!(result.is_err(), "invalid YAML should fail to parse");
    }

    #[test]
    fn test_swagger2_endpoints() {
        let yaml = r#"
swagger: "2.0"
info:
  title: API Service
  version: "1.0.0"
paths:
  /users:
    get:
      operationId: listUsers
    post:
      operationId: createUser
"#;

        let (_service, endpoints, _schemas, _connections) =
            parse_swagger_2(yaml, "swagger.yaml").expect("parse failed");

        assert_eq!(endpoints.len(), 2);
        assert!(endpoints
            .iter()
            .any(|e| e.method == "GET" && e.path == "/users"));
        assert!(endpoints
            .iter()
            .any(|e| e.method == "POST" && e.path == "/users"));

        for endpoint in &endpoints {
            assert_eq!(endpoint.extraction_method, "spec:openapi");
        }
    }

    #[test]
    fn test_plugin_file_patterns() {
        let plugin = OpenApiPlugin;
        let patterns = plugin.file_patterns();
        assert!(patterns.contains(&"**/openapi.yaml"));
        assert!(patterns.contains(&"**/swagger.json"));
    }

    #[test]
    fn test_plugin_always_run() {
        let plugin = OpenApiPlugin;
        assert!(plugin.always_run());
    }

    #[test]
    fn test_oas3_servers_extraction() {
        let yaml = r#"
openapi: "3.0.3"
info:
  title: My Service
  version: "1.0.0"
servers:
  - url: "https://api.example.com/v2"
paths: {}
"#;

        let (_service, _endpoints, _schemas, connections) =
            parse_openapi_3(yaml, "test.yaml").expect("parse failed");

        assert_eq!(connections.len(), 1);
        let conn = &connections[0];
        assert_eq!(conn.protocol, "http");
        assert_eq!(conn.target_name, "api.example.com");
        assert_eq!(conn.confidence, Confidence::Medium);
        assert_eq!(conn.extraction_method, "spec:openapi");
    }

    #[test]
    fn test_oas3_servers_template_skipped() {
        let yaml = r#"
openapi: "3.0.3"
info:
  title: My Service
  version: "1.0.0"
servers:
  - url: "https://{server}/v2"
paths: {}
"#;

        let (_service, _endpoints, _schemas, connections) =
            parse_openapi_3(yaml, "test.yaml").expect("parse failed");

        assert_eq!(connections.len(), 0);
    }

    #[test]
    fn test_oas3_multiple_servers() {
        let yaml = r#"
openapi: "3.0.3"
info:
  title: My Service
  version: "1.0.0"
servers:
  - url: "https://api.example.com/v1"
  - url: "https://staging.example.com/v1"
paths: {}
"#;

        let (_service, _endpoints, _schemas, connections) =
            parse_openapi_3(yaml, "test.yaml").expect("parse failed");

        assert_eq!(connections.len(), 2);
    }

    #[test]
    fn test_swagger2_host_extraction() {
        let yaml = r#"
swagger: "2.0"
info:
  title: My API
  version: "1.0.0"
host: "api.example.com"
paths: {}
"#;

        let (_service, _endpoints, _schemas, connections) =
            parse_swagger_2(yaml, "swagger.yaml").expect("parse failed");

        assert_eq!(connections.len(), 1);
        let conn = &connections[0];
        assert_eq!(conn.target_name, "api.example.com");
        assert_eq!(conn.protocol, "http");
    }

    #[test]
    fn test_swagger2_host_with_port() {
        let yaml = r#"
swagger: "2.0"
info:
  title: My API
  version: "1.0.0"
host: "api.example.com:8080"
paths: {}
"#;

        let (_service, _endpoints, _schemas, connections) =
            parse_swagger_2(yaml, "swagger.yaml").expect("parse failed");

        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].target_name, "api.example.com");
    }

    #[test]
    fn test_swagger2_no_host() {
        let yaml = r#"
swagger: "2.0"
info:
  title: My API
  version: "1.0.0"
paths: {}
"#;

        let (_service, _endpoints, _schemas, connections) =
            parse_swagger_2(yaml, "swagger.yaml").expect("parse failed");

        assert_eq!(connections.len(), 0);
    }
}
