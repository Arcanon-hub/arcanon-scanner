use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, EndpointInfo, ExtractionResult, FieldInfo, SchemaInfo};
use std::path::Path;
use tracing::warn;

/// GraphQL schema and operation parser.
pub struct GraphqlPlugin;

impl LanguagePlugin for GraphqlPlugin {
    fn name(&self) -> &str {
        "graphql"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.graphql", "**/*.gql"]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        for file in &ctx.files {
            match parse_graphql(&file.content, &file.relative_path) {
                Ok((endpoints, schemas)) => {
                    result.endpoints.extend(endpoints);
                    result.schemas.extend(schemas);
                }
                Err(e) => {
                    warn!("graphql: failed to parse {}: {}", file.relative_path, e);
                }
            }
        }

        result
    }
}

/// Parse a GraphQL schema file using simple text parsing
fn parse_graphql(
    content: &str,
    relative_path: &str,
) -> Result<(Vec<EndpointInfo>, Vec<SchemaInfo>), String> {
    let mut endpoints = Vec::new();
    let mut schemas = Vec::new();

    // Get service name from file stem
    let service_name = Path::new(relative_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("graphql")
        .to_string();

    // Simple line-by-line parsing
    let lines: Vec<&str> = content.lines().collect();
    let mut line_idx = 0;

    while line_idx < lines.len() {
        let line = lines[line_idx].trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with("#") {
            line_idx += 1;
            continue;
        }

        // Check for type definitions (type Query, type Mutation, type User, etc.)
        if (line.starts_with("type ") || line.starts_with("input ")) && line.ends_with("{") {
            let type_def = if line.starts_with("type ") {
                line.strip_prefix("type ").unwrap_or("").trim()
            } else {
                line.strip_prefix("input ").unwrap_or("").trim()
            };

            let type_name = type_def.strip_suffix(" {").unwrap_or("").trim().to_string();

            // Special handling for Query, Mutation, Subscription
            let is_operation_type =
                type_name == "Query" || type_name == "Mutation" || type_name == "Subscription";

            let mut fields = Vec::new();
            line_idx += 1;

            // Parse fields until closing brace
            while line_idx < lines.len() {
                let field_line = lines[line_idx].trim();

                if field_line == "}" {
                    break;
                }

                if !field_line.is_empty() && !field_line.starts_with("#") {
                    // Extract field name
                    if let Some(field_name) = extract_field_name(field_line) {
                        // For operation types (Query, Mutation), treat fields as endpoints
                        if is_operation_type {
                            endpoints.push(EndpointInfo {
                                service_name: service_name.clone(),
                                method: type_name.to_lowercase(),
                                path: field_name.clone(),
                                handler: Some(field_name.clone()),
                                kind: "graphql".to_string(),
                                confidence: Confidence::High,
                                extraction_method: "spec:graphql".to_string(),
                            });
                        } else {
                            // Regular type definitions get fields
                            fields.push(FieldInfo {
                                name: field_name,
                                field_type: "unknown".to_string(),
                                required: false,
                            });
                        }
                    }
                }

                line_idx += 1;
            }

            // Add schema if not an operation type
            if !is_operation_type {
                schemas.push(SchemaInfo {
                    name: type_name,
                    role: "type".to_string(),
                    file: Some(relative_path.to_string()),
                    connection_ref: None,
                    fields,
                    confidence: Confidence::High,
                    extraction_method: "spec:graphql".to_string(),
                });
            }
        }

        line_idx += 1;
    }

    Ok((endpoints, schemas))
}

/// Extract field name from a GraphQL field definition line
fn extract_field_name(line: &str) -> Option<String> {
    // Format: fieldName(args...): Type
    // or: fieldName: Type
    if let Some(paren_pos) = line.find('(') {
        // Has arguments
        Some(line[..paren_pos].trim().to_string())
    } else {
        line.find(':')
            .map(|colon_pos| line[..colon_pos].trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graphql_query_extraction() {
        let schema = r#"
type Query {
  getUserById(id: ID!): User
  listUsers: [User!]!
}

type User {
  id: ID!
  email: String!
}
"#;

        let (endpoints, schemas) = parse_graphql(schema, "schema.graphql").expect("parse failed");

        // Should extract query operations as endpoints
        assert!(endpoints
            .iter()
            .any(|e| e.method == "query" && e.path == "getUserById"));
        assert!(endpoints
            .iter()
            .any(|e| e.method == "query" && e.path == "listUsers"));

        // Should extract User type as schema
        assert!(schemas.iter().any(|s| s.name == "User"));
    }

    #[test]
    fn test_graphql_mutation_extraction() {
        let schema = r#"
type Mutation {
  createUser(input: CreateUserInput!): User!
  deleteUser(id: ID!): Boolean!
}

type User {
  id: ID!
}
"#;

        let (endpoints, _schemas) = parse_graphql(schema, "schema.graphql").expect("parse failed");

        // Should have mutation endpoints
        assert!(endpoints.iter().any(|e| e.method == "mutation"));
        assert_eq!(
            endpoints.iter().filter(|e| e.method == "mutation").count(),
            2
        );
    }

    #[test]
    fn test_graphql_type_extraction() {
        let schema = r#"
type User {
  id: ID!
  email: String!
  name: String!
}

type Post {
  id: ID!
  title: String!
}
"#;

        let (_endpoints, schemas) = parse_graphql(schema, "schema.graphql").expect("parse failed");

        // Should extract User and Post type definitions
        assert_eq!(schemas.len(), 2);
        let user_schema = schemas.iter().find(|s| s.name == "User");
        assert!(user_schema.is_some());

        let post_schema = schemas.iter().find(|s| s.name == "Post");
        assert!(post_schema.is_some());

        for schema in &schemas {
            assert_eq!(schema.role, "type");
            assert_eq!(schema.extraction_method, "spec:graphql");
        }
    }

    #[test]
    fn test_graphql_invalid_content() {
        let invalid = "{{{this is not valid graphql";
        let (endpoints, schemas) =
            parse_graphql(invalid, "schema.graphql").expect("parse should not fail");
        // Simple parser may handle this gracefully
        assert_eq!(endpoints.len() + schemas.len(), 0);
    }

    #[test]
    fn test_plugin_file_patterns() {
        let plugin = GraphqlPlugin;
        let patterns = plugin.file_patterns();
        assert!(patterns.contains(&"**/*.graphql"));
        assert!(patterns.contains(&"**/*.gql"));
    }

    #[test]
    fn test_plugin_always_run() {
        let plugin = GraphqlPlugin;
        assert!(plugin.always_run());
    }
}
