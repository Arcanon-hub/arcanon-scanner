use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, EndpointInfo, ExtractionResult, FieldInfo, SchemaInfo};
use tracing::warn;

/// Protocol Buffers (.proto) file parser (gRPC service extraction).
pub struct ProtoPlugin;

impl LanguagePlugin for ProtoPlugin {
    fn name(&self) -> &str {
        "proto"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.proto"]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        for file in &ctx.files {
            match parse_proto(&file.content, &file.relative_path) {
                Ok((endpoints, schemas)) => {
                    result.endpoints.extend(endpoints);
                    result.schemas.extend(schemas);
                }
                Err(e) => {
                    warn!("proto: failed to parse {}: {}", file.relative_path, e);
                }
            }
        }

        result
    }
}

/// Parse a .proto file using simple text parsing
fn parse_proto(
    content: &str,
    relative_path: &str,
) -> Result<(Vec<EndpointInfo>, Vec<SchemaInfo>), String> {
    let mut endpoints = Vec::new();
    let mut schemas = Vec::new();
    let mut current_service: Option<String> = None;
    let mut current_message: Option<CurrentMessage> = None;
    let mut in_service_block = false;
    let mut in_message_block = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments and empty lines
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }

        // Check for service definition
        if trimmed.starts_with("service ") && trimmed.ends_with("{") {
            let service_name = trimmed
                .strip_prefix("service ")
                .and_then(|s| s.strip_suffix(" {"))
                .unwrap_or("")
                .trim()
                .to_string();
            if !service_name.is_empty() {
                current_service = Some(service_name);
                in_service_block = true;
            }
        } else if trimmed == "}" {
            // Closing brace
            if in_service_block {
                in_service_block = false;
                current_service = None;
            } else if in_message_block {
                in_message_block = false;
                if let Some(msg) = current_message.take() {
                    schemas.push(SchemaInfo {
                        name: msg.name,
                        role: "type".to_string(),
                        file: Some(relative_path.to_string()),
                        connection_ref: None,
                        fields: msg.fields,
                        confidence: Confidence::High,
                        extraction_method: "spec:proto".to_string(),
                        ..Default::default()
                    });
                }
            }
        } else if trimmed.starts_with("message ") && trimmed.ends_with("{") {
            // Message definition
            let message_name = trimmed
                .strip_prefix("message ")
                .and_then(|s| s.strip_suffix(" {"))
                .unwrap_or("")
                .trim()
                .to_string();
            if !message_name.is_empty() {
                current_message = Some(CurrentMessage {
                    name: message_name,
                    fields: Vec::new(),
                });
                in_message_block = true;
            }
        } else if in_service_block {
            // Parse rpc inside service
            if trimmed.starts_with("rpc ") {
                if let Some(service_name) = &current_service {
                    // Parse: rpc GetUser(GetUserRequest) returns (GetUserResponse);
                    if let Some(method_name) = parse_rpc_method(trimmed) {
                        endpoints.push(EndpointInfo {
                            service_name: service_name.clone(),
                            method: "rpc".to_string(),
                            path: format!("{}/{}", service_name, method_name),
                            handler: Some(method_name),
                            kind: "grpc".to_string(),
                            confidence: Confidence::High,
                            extraction_method: "spec:proto".to_string(),
                            ..Default::default()
                        });
                    }
                }
            }
        } else if in_message_block {
            // Parse field inside message
            if let Some(msg) = &mut current_message {
                if let Some(field) = parse_field(trimmed) {
                    msg.fields.push(field);
                }
            }
        }
    }

    Ok((endpoints, schemas))
}

/// Parse an rpc method definition and return the method name
fn parse_rpc_method(line: &str) -> Option<String> {
    // Format: rpc GetUser(GetUserRequest) returns (GetUserResponse);
    if let Some(start) = line.find("rpc ") {
        let rest = &line[start + 4..];
        if let Some(end) = rest.find('(') {
            return Some(rest[..end].trim().to_string());
        }
    }
    None
}

/// Parse a field definition and return FieldInfo
fn parse_field(line: &str) -> Option<FieldInfo> {
    // Format: string user_id = 1;
    // Split by whitespace to get: [type, name, =, number;]
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() >= 2 {
        let field_type = parts[0].to_string();
        let field_name = parts[1].to_string();
        return Some(FieldInfo {
            name: field_name,
            field_type,
            required: false,
        });
    }
    None
}

/// Temporary struct to collect message info while parsing
struct CurrentMessage {
    name: String,
    fields: Vec<FieldInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proto_service_parsing() {
        let proto = r#"
syntax = "proto3";

package userservice;

service UserService {
  rpc GetUser(GetUserRequest) returns (GetUserResponse);
  rpc CreateUser(CreateUserRequest) returns (CreateUserResponse);
}

message GetUserRequest {
  string user_id = 1;
}

message GetUserResponse {
  string user_id = 1;
  string email = 2;
  string name = 3;
}

message CreateUserRequest {
  string email = 1;
  string name = 2;
}

message CreateUserResponse {
  string user_id = 1;
}
"#;

        let (endpoints, schemas) = parse_proto(proto, "service.proto").expect("parse failed");

        // Check endpoints
        assert_eq!(endpoints.len(), 2);
        assert!(endpoints.iter().any(|e| {
            e.service_name == "UserService"
                && e.handler.as_ref().map(|h| h.as_str()) == Some("GetUser")
        }));
        assert!(endpoints.iter().any(|e| {
            e.service_name == "UserService"
                && e.handler.as_ref().map(|h| h.as_str()) == Some("CreateUser")
        }));

        for endpoint in &endpoints {
            assert_eq!(endpoint.kind, "grpc");
            assert_eq!(endpoint.method, "rpc");
            assert_eq!(endpoint.extraction_method, "spec:proto");
            assert_eq!(endpoint.confidence, Confidence::High);
        }

        // Check schemas
        assert_eq!(schemas.len(), 4);
        let get_request = schemas.iter().find(|s| s.name == "GetUserRequest").unwrap();
        assert_eq!(get_request.fields.len(), 1);
        assert_eq!(get_request.fields[0].name, "user_id");

        let get_response = schemas
            .iter()
            .find(|s| s.name == "GetUserResponse")
            .unwrap();
        assert_eq!(get_response.fields.len(), 3);
    }

    #[test]
    fn test_proto_schema_extraction() {
        let proto = r#"
syntax = "proto3";

message User {
  string id = 1;
  string email = 2;
  string name = 3;
}

message Address {
  string street = 1;
  string city = 2;
}
"#;

        let (_endpoints, schemas) = parse_proto(proto, "models.proto").expect("parse failed");

        assert_eq!(schemas.len(), 2);

        let user = schemas.iter().find(|s| s.name == "User").unwrap();
        assert_eq!(user.fields.len(), 3);
        assert!(user.fields.iter().any(|f| f.name == "email"));
        assert_eq!(user.extraction_method, "spec:proto");
    }

    #[test]
    fn test_proto_invalid_content() {
        let invalid = "this is not valid proto";
        let (endpoints, schemas) =
            parse_proto(invalid, "bad.proto").expect("parse should not fail");
        assert_eq!(endpoints.len(), 0);
        assert_eq!(schemas.len(), 0);
    }

    #[test]
    fn test_plugin_file_patterns() {
        let plugin = ProtoPlugin;
        let patterns = plugin.file_patterns();
        assert!(patterns.contains(&"**/*.proto"));
    }

    #[test]
    fn test_plugin_always_run() {
        let plugin = ProtoPlugin;
        assert!(plugin.always_run());
    }
}
