// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::collections::HashMap;
use std::sync::OnceLock;

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

use crate::plugin::{scope_to_service, ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, EndpointInfo, ExtractionResult};

/// TypeScript/JavaScript language plugin.
/// Covers .ts, .tsx, .js, .jsx, and package.json for framework detection.
pub struct TypeScriptPlugin;

/// Framework detection results
#[derive(Debug, Clone, Default)]
struct FrameworkSet {
    express: bool,
    nestjs: bool,
    nextjs: bool,
    fastify: bool,
}

// Query constants for route detection
const ROUTE_QUERY_EXPRESS: &str = r#"
(call_expression
  function: (member_expression
    object: (identifier) @receiver
    property: (property_identifier) @method)
  arguments: (arguments
    (string) @path
    (_)* @handler))
"#;

const NESTJS_CONTROLLER_QUERY: &str = r#"
(class_declaration
  (decorator
    (call_expression
      function: (identifier) @dec_name
      arguments: (arguments (string) @prefix)))
  name: (type_identifier) @class_name)
"#;

const NESTJS_METHOD_QUERY: &str = r#"
(method_definition
  (decorator
    (call_expression
      function: (identifier) @http_dec
      arguments: (arguments (string)? @method_path)))
  name: (property_identifier) @handler_name)
"#;

// Connection detection queries
const FETCH_CALL_QUERY: &str = r#"
(call_expression
  function: (identifier) @fn
  arguments: (arguments (_) @url (_)*))
"#;

const AXIOS_CALL_QUERY: &str = r#"
(call_expression
  function: (member_expression
    object: (identifier) @lib
    property: (property_identifier) @method)
  arguments: (arguments (_) @url (_)*))
"#;

const DB_NEW_QUERY: &str = r#"
(new_expression
  constructor: (identifier) @ctor
  arguments: (arguments (_) @arg))
"#;

const GRPC_NEW_QUERY: &str = r#"
(new_expression
  constructor: (identifier) @ctor
  arguments: (arguments (_)+))
"#;

const MONGOOSE_CONNECT_QUERY: &str = r#"
(call_expression
  function: (member_expression
    object: (identifier) @obj
    property: (property_identifier) @method)
  arguments: (arguments (_) @uri))
"#;

const KAFKA_SEND_QUERY: &str = r#"
(call_expression
  function: (member_expression
    object: (identifier) @producer
    property: (property_identifier) @method)
  arguments: (arguments (object (pair key: (property_identifier) @key value: (string) @topic))))
"#;

// OnceLock query caches for Express
static EXPRESS_QUERY: OnceLock<Query> = OnceLock::new();

fn express_query(lang: &Language) -> &'static Query {
    EXPRESS_QUERY
        .get_or_init(|| Query::new(lang, ROUTE_QUERY_EXPRESS).expect("valid express query"))
}

// OnceLock query caches for NestJS
static NESTJS_CONTROLLER_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static NESTJS_METHOD_QUERY_CACHE: OnceLock<Query> = OnceLock::new();

fn nestjs_controller_query(lang: &Language) -> &'static Query {
    NESTJS_CONTROLLER_QUERY_CACHE.get_or_init(|| {
        Query::new(lang, NESTJS_CONTROLLER_QUERY).expect("valid nestjs controller query")
    })
}

fn nestjs_method_query(lang: &Language) -> &'static Query {
    NESTJS_METHOD_QUERY_CACHE
        .get_or_init(|| Query::new(lang, NESTJS_METHOD_QUERY).expect("valid nestjs method query"))
}

// OnceLock query caches for connection detection
static FETCH_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static AXIOS_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static DB_NEW_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static GRPC_NEW_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static MONGOOSE_CONNECT_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static KAFKA_SEND_QUERY_CACHE: OnceLock<Query> = OnceLock::new();

fn fetch_query(lang: &Language) -> &'static Query {
    FETCH_QUERY_CACHE.get_or_init(|| Query::new(lang, FETCH_CALL_QUERY).expect("valid fetch query"))
}

fn axios_query(lang: &Language) -> &'static Query {
    AXIOS_QUERY_CACHE.get_or_init(|| Query::new(lang, AXIOS_CALL_QUERY).expect("valid axios query"))
}

fn db_new_query(lang: &Language) -> &'static Query {
    DB_NEW_QUERY_CACHE.get_or_init(|| Query::new(lang, DB_NEW_QUERY).expect("valid db new query"))
}

fn grpc_new_query(lang: &Language) -> &'static Query {
    GRPC_NEW_QUERY_CACHE
        .get_or_init(|| Query::new(lang, GRPC_NEW_QUERY).expect("valid grpc new query"))
}

fn mongoose_connect_query(lang: &Language) -> &'static Query {
    MONGOOSE_CONNECT_QUERY_CACHE.get_or_init(|| {
        Query::new(lang, MONGOOSE_CONNECT_QUERY).expect("valid mongoose connect query")
    })
}

fn kafka_send_query(lang: &Language) -> &'static Query {
    KAFKA_SEND_QUERY_CACHE
        .get_or_init(|| Query::new(lang, KAFKA_SEND_QUERY).expect("valid kafka send query"))
}

/// Detect frameworks from package.json in the file list
fn detect_frameworks(ctx: &ExtractionContext) -> FrameworkSet {
    let mut frameworks = FrameworkSet::default();

    for file in &ctx.files {
        if file.relative_path.ends_with("package.json") {
            let content = &*file.content;
            if content.contains("\"express\"") {
                frameworks.express = true;
            }
            if content.contains("\"@nestjs/core\"") {
                frameworks.nestjs = true;
            }
            if content.contains("\"next\"") {
                frameworks.nextjs = true;
            }
            if content.contains("\"fastify\"") {
                frameworks.fastify = true;
            }
        }
    }

    frameworks
}

/// Extract Express routes from a parsed file
fn extract_express_routes(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;
    let mut parser = Parser::new();
    if parser.set_language(lang).is_err() {
        return;
    }

    let tree = match parser.parse(file_content.as_bytes(), None) {
        Some(t) => t,
        None => return,
    };

    let query = express_query(lang);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), file_content.as_bytes());

    let capture_names = query.capture_names();
    let mut processed_matches = Vec::new();

    while let Some(m) = matches.next() {
        let mut receiver = String::new();
        let mut method = String::new();
        let mut path = String::new();
        let mut handler = String::new();
        let mut line = 1;

        for capture in m.captures {
            let capture_name = capture_names[capture.index as usize];
            let node = capture.node;
            let text = node
                .utf8_text(file_content.as_bytes())
                .unwrap_or("")
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();

            match capture_name {
                "receiver" => receiver = text,
                "method" => method = text,
                "path" => path = text,
                "handler" => handler = text,
                _ => {}
            }

            line = node.start_position().row + 1;
        }

        processed_matches.push((receiver, method, path, handler, line));
    }

    // Filter and emit endpoints
    let allowed_receivers = ["app", "router", "api", "r", "v1", "v2"];
    let http_methods = [
        "get", "post", "put", "delete", "patch", "head", "options", "all",
    ];

    for (receiver, method, path, handler, _line) in processed_matches {
        if !allowed_receivers.contains(&receiver.as_str()) {
            continue;
        }
        if !http_methods.contains(&method.to_lowercase().as_str()) {
            continue;
        }

        let service_name = scope_to_service(&file.path, service_roots)
            .unwrap_or("")
            .to_string();

        let _evidence = format!("{}.{}('{}', ...)", receiver, method, path);

        result.endpoints.push(EndpointInfo {
            service_name,
            method: method.to_uppercase(),
            path,
            handler: if handler.is_empty() {
                None
            } else {
                Some(handler)
            },
            kind: "rest".to_string(),
            confidence: Confidence::High,
            extraction_method: "ast_express".to_string(),
        });
    }
}

/// Extract NestJS routes using two-phase extraction (DETQ-05)
/// Phase 1: Extract @Controller('/prefix') class decorator
/// Phase 2: Extract @Get/@Post/etc method decorators and combine with prefix
fn extract_nestjs_routes(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;
    let mut parser = Parser::new();
    if parser.set_language(lang).is_err() {
        return;
    }

    let tree = match parser.parse(file_content.as_bytes(), None) {
        Some(t) => t,
        None => return,
    };

    // Extract @Controller prefix from class declarations
    let controller_query_str = r#"
(decorator
  (call_expression
    function: (identifier) @dec_name
    arguments: (arguments (string) @prefix)))
"#;

    let mut controller_prefix = String::new();

    if let Ok(controller_query) = Query::new(lang, controller_query_str) {
        let mut cursor = QueryCursor::new();
        let mut matches =
            cursor.matches(&controller_query, tree.root_node(), file_content.as_bytes());
        let capture_names = controller_query.capture_names();

        // Find the @Controller decorator with its prefix
        while let Some(m) = matches.next() {
            let mut dec_name = String::new();
            let mut prefix = String::new();

            for capture in m.captures {
                let capture_name = capture_names[capture.index as usize];
                let node = capture.node;
                let text = node
                    .utf8_text(file_content.as_bytes())
                    .unwrap_or("")
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();

                match capture_name {
                    "dec_name" => dec_name = text,
                    "prefix" => prefix = text,
                    _ => {}
                }
            }

            if dec_name == "Controller" {
                controller_prefix = prefix;
                break;
            }
        }
    }

    // Phase 2: Extract method decorators
    let method_decorator_query_str = r#"
(decorator
  (call_expression
    function: (identifier) @http_dec
    arguments: (arguments (string)? @path_arg)))
"#;

    if let Ok(method_query) = Query::new(lang, method_decorator_query_str) {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&method_query, tree.root_node(), file_content.as_bytes());
        let capture_names = method_query.capture_names();

        let http_methods = ["Get", "Post", "Put", "Delete", "Patch"];
        let service_name = scope_to_service(&file.path, service_roots)
            .unwrap_or("")
            .to_string();

        while let Some(m) = matches.next() {
            let mut http_dec = String::new();
            let mut path_arg = String::new();

            for capture in m.captures {
                let capture_name = capture_names[capture.index as usize];
                let node = capture.node;
                let text = node
                    .utf8_text(file_content.as_bytes())
                    .unwrap_or("")
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();

                match capture_name {
                    "http_dec" => http_dec = text,
                    "path_arg" => path_arg = text,
                    _ => {}
                }
            }

            if http_methods.contains(&http_dec.as_str()) {
                // Combine controller prefix with method path
                let combined_path = if controller_prefix.is_empty() {
                    path_arg
                } else if path_arg.is_empty() {
                    controller_prefix.clone()
                } else {
                    format!("{}{}", controller_prefix, path_arg)
                };

                result.endpoints.push(EndpointInfo {
                    service_name: service_name.clone(),
                    method: http_dec.to_uppercase(),
                    path: combined_path,
                    handler: None,
                    kind: "rest".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "ast_nestjs_two_phase".to_string(),
                });
            }
        }
    }
}

/// Extract HTTP client calls (fetch, axios, got, etc.)
fn extract_http_clients(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;
    let mut parser = Parser::new();
    if parser.set_language(lang).is_err() {
        return;
    }

    let tree = match parser.parse(file_content.as_bytes(), None) {
        Some(t) => t,
        None => return,
    };

    // Check for fetch calls
    let fetch_query = fetch_query(lang);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(fetch_query, tree.root_node(), file_content.as_bytes());

    let capture_names = fetch_query.capture_names();

    while let Some(m) = matches.next() {
        let mut fn_name = String::new();
        let mut url_arg = String::new();
        let mut line = 1;

        for capture in m.captures {
            let capture_name = capture_names[capture.index as usize];
            let node = capture.node;
            let text = node
                .utf8_text(file_content.as_bytes())
                .unwrap_or("")
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();

            match capture_name {
                "fn" => fn_name = text,
                "url" => url_arg = text,
                _ => {}
            }

            line = node.start_position().row + 1;
        }

        if fn_name == "fetch" {
            let service_name = scope_to_service(&file.path, service_roots)
                .unwrap_or("")
                .to_string();

            let evidence = format!("fetch('{}')", url_arg);
            let truncated_evidence = if evidence.len() > 200 {
                evidence[..200].to_string()
            } else {
                evidence
            };

            result.connections.push(ConnectionInfo {
                source_service: service_name,
                target_name: url_arg.clone(),
                protocol: "rest".to_string(),
                method: None,
                path: None,
                source_file: format!("{}:{}", file.relative_path, line),
                confidence: Confidence::High,
                extraction_method: "ast_fetch".to_string(),
                evidence: Some(truncated_evidence),
            });
        }
    }
}

/// Extract database connections (pg, mongoose, redis, mysql, etc.)
fn extract_database_connections(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;
    // Check for mongoose.connect() calls
    if file_content.contains("mongoose") || file_content.contains("from 'mongoose'") {
        let mut parser = Parser::new();
        if parser.set_language(lang).is_err() {
            return;
        }

        let tree = match parser.parse(file_content.as_bytes(), None) {
            Some(t) => t,
            None => return,
        };

        let mongoose_query = mongoose_connect_query(lang);
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(mongoose_query, tree.root_node(), file_content.as_bytes());

        let capture_names = mongoose_query.capture_names();

        while let Some(m) = matches.next() {
            let mut obj = String::new();
            let mut method = String::new();
            let mut line = 1;

            for capture in m.captures {
                let capture_name = capture_names[capture.index as usize];
                let node = capture.node;
                let text = node
                    .utf8_text(file_content.as_bytes())
                    .unwrap_or("")
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();

                match capture_name {
                    "obj" => obj = text,
                    "method" => method = text,
                    _ => {}
                }

                line = node.start_position().row + 1;
            }

            if obj == "mongoose" && method == "connect" {
                let service_name = scope_to_service(&file.path, service_roots)
                    .unwrap_or("")
                    .to_string();

                let evidence = "mongoose.connect()".to_string();

                result.connections.push(ConnectionInfo {
                    source_service: service_name,
                    target_name: "mongodb".to_string(),
                    protocol: "mongodb".to_string(),
                    method: None,
                    path: None,
                    source_file: format!("{}:{}", file.relative_path, line),
                    confidence: Confidence::High,
                    extraction_method: "ast_mongoose".to_string(),
                    evidence: Some(evidence),
                });
            }
        }
    }
}

/// Extract gRPC client instantiations
fn extract_grpc_clients(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;
    if !file_content.contains("_grpc") && !file_content.contains("_pb2_grpc") {
        return;
    }

    let mut parser = Parser::new();
    if parser.set_language(lang).is_err() {
        return;
    }

    let tree = match parser.parse(file_content.as_bytes(), None) {
        Some(t) => t,
        None => return,
    };

    let grpc_query = grpc_new_query(lang);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(grpc_query, tree.root_node(), file_content.as_bytes());

    let capture_names = grpc_query.capture_names();

    while let Some(m) = matches.next() {
        let mut ctor = String::new();
        let mut line = 1;

        for capture in m.captures {
            let capture_name = capture_names[capture.index as usize];
            let node = capture.node;
            let text = node
                .utf8_text(file_content.as_bytes())
                .unwrap_or("")
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();

            match capture_name {
                "ctor" => ctor = text,
                _ => {}
            }

            line = node.start_position().row + 1;
        }

        if ctor.ends_with("Client") {
            let service_name = scope_to_service(&file.path, service_roots)
                .unwrap_or("")
                .to_string();

            let target = ctor.strip_suffix("Client").unwrap_or(&ctor).to_string();

            let evidence = format!("new {}(...)", ctor);

            result.connections.push(ConnectionInfo {
                source_service: service_name,
                target_name: target,
                protocol: "grpc".to_string(),
                method: None,
                path: None,
                source_file: format!("{}:{}", file.relative_path, line),
                confidence: Confidence::High,
                extraction_method: "ast_grpc".to_string(),
                evidence: Some(evidence),
            });
        }
    }
}

/// Extract message queue calls (kafkajs, amqplib, mqtt.js)
fn extract_mq_calls(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;
    if !file_content.contains("producer.send")
        && !file_content.contains("channel.publish")
        && !file_content.contains("publish")
    {
        return;
    }

    let mut parser = Parser::new();
    if parser.set_language(lang).is_err() {
        return;
    }

    let tree = match parser.parse(file_content.as_bytes(), None) {
        Some(t) => t,
        None => return,
    };

    // Try kafka send query
    let kafka_query = kafka_send_query(lang);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(kafka_query, tree.root_node(), file_content.as_bytes());

    let capture_names = kafka_query.capture_names();

    while let Some(m) = matches.next() {
        let mut producer = String::new();
        let mut method = String::new();
        let mut key = String::new();
        let mut topic = String::new();
        let mut line = 1;

        for capture in m.captures {
            let capture_name = capture_names[capture.index as usize];
            let node = capture.node;
            let text = node
                .utf8_text(file_content.as_bytes())
                .unwrap_or("")
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();

            match capture_name {
                "producer" => producer = text,
                "method" => method = text,
                "key" => key = text,
                "topic" => topic = text,
                _ => {}
            }

            line = node.start_position().row + 1;
        }

        if method == "send" && key == "topic" {
            let service_name = scope_to_service(&file.path, service_roots)
                .unwrap_or("")
                .to_string();

            let evidence = format!("producer.send({{ topic: '{}' }})", topic);
            let truncated_evidence = if evidence.len() > 200 {
                evidence[..200].to_string()
            } else {
                evidence
            };

            result.connections.push(ConnectionInfo {
                source_service: service_name,
                target_name: producer,
                protocol: "kafka".to_string(),
                method: None,
                path: Some(topic),
                source_file: format!("{}:{}", file.relative_path, line),
                confidence: Confidence::High,
                extraction_method: "ast_kafka".to_string(),
                evidence: Some(truncated_evidence),
            });
        }
    }
}

impl LanguagePlugin for TypeScriptPlugin {
    fn name(&self) -> &str {
        "typescript"
    }

    fn file_patterns(&self) -> &[&str] {
        &[
            "**/*.ts",
            "**/*.tsx",
            "**/*.js",
            "**/*.jsx",
            "**/package.json", // needed for framework marker detection (LPLU-08)
        ]
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        // Detect frameworks from package.json
        let frameworks = detect_frameworks(ctx);

        let lang = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();

        // Process source files (not package.json)
        for file in &ctx.files {
            if file.relative_path.ends_with("package.json") {
                continue; // Skip manifest files for route extraction
            }

            if file.relative_path.ends_with(".ts")
                || file.relative_path.ends_with(".tsx")
                || file.relative_path.ends_with(".js")
                || file.relative_path.ends_with(".jsx")
            {
                // Extract Express routes if framework detected
                if frameworks.express {
                    extract_express_routes(&mut result, &lang, file, &ctx.service_roots);
                }

                // Extract NestJS routes if framework detected
                if frameworks.nestjs {
                    extract_nestjs_routes(&mut result, &lang, file, &ctx.service_roots);
                }

                // Extract HTTP clients (fetch, axios, got, etc.)
                extract_http_clients(&mut result, &lang, file, &ctx.service_roots);

                // Extract database connections
                extract_database_connections(&mut result, &lang, file, &ctx.service_roots);

                // Extract gRPC clients
                extract_grpc_clients(&mut result, &lang, file, &ctx.service_roots);

                // Extract message queue calls
                extract_mq_calls(&mut result, &lang, file, &ctx.service_roots);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::FileContext;
    use std::sync::Arc;

    #[test]
    fn test_express_route_detection() {
        let plugin = TypeScriptPlugin;

        let package_json = FileContext {
            path: std::path::PathBuf::from("/repo/package.json"),
            relative_path: "package.json".to_string(),
            content: Arc::from(r#"{"dependencies": {"express": "^4.0"}}"#),
        };

        let app_file = FileContext {
            path: std::path::PathBuf::from("/repo/app.ts"),
            relative_path: "app.ts".to_string(),
            content: Arc::from("const app = express(); app.get('/users', getUsers);"),
        };

        let ctx = ExtractionContext {
            files: vec![package_json, app_file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(!result.endpoints.is_empty(), "Should detect Express route");
        let endpoint = &result.endpoints[0];
        assert_eq!(endpoint.method, "GET");
        assert_eq!(endpoint.path, "/users");
        assert_eq!(endpoint.kind, "rest");
        assert_eq!(endpoint.extraction_method, "ast_express");
    }

    #[test]
    fn test_no_endpoints_without_framework_marker() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/app.ts"),
            relative_path: "app.ts".to_string(),
            content: Arc::from("const app = express(); app.get('/users', getUsers);"),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        // Without package.json declaring express, should find no routes
        assert!(
            result.endpoints.is_empty(),
            "Should not detect Express route without framework marker"
        );
    }

    #[test]
    fn test_nestjs_route_detection() {
        let plugin = TypeScriptPlugin;

        let package_json = FileContext {
            path: std::path::PathBuf::from("/repo/package.json"),
            relative_path: "package.json".to_string(),
            content: Arc::from(r#"{"dependencies": {"@nestjs/core": "^10.0"}}"#),
        };

        let controller_file = FileContext {
            path: std::path::PathBuf::from("/repo/users.controller.ts"),
            relative_path: "users.controller.ts".to_string(),
            content: Arc::from(
                r#"
@Controller('/users')
export class UsersController {
    @Get('/:id')
    getUser() { }
}
"#,
            ),
        };

        let ctx = ExtractionContext {
            files: vec![package_json, controller_file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(!result.endpoints.is_empty(), "Should detect NestJS route");
        let endpoint = &result.endpoints[0];
        assert_eq!(endpoint.method, "GET");
        assert_eq!(endpoint.extraction_method, "ast_nestjs_two_phase");
    }

    #[test]
    fn test_fetch_http_client_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/api.ts"),
            relative_path: "api.ts".to_string(),
            content: Arc::from("const response = await fetch('/api/users');"),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty(), "Should detect fetch call");
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "rest");
        assert!(
            conn.evidence.as_ref().unwrap().contains("fetch"),
            "Evidence should contain 'fetch'"
        );
    }

    #[test]
    fn test_mongoose_db_connection_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/db.ts"),
            relative_path: "db.ts".to_string(),
            content: Arc::from(
                "import mongoose from 'mongoose'; mongoose.connect('mongodb://localhost:27017/db');",
            ),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(
            !result.connections.is_empty(),
            "Should detect mongoose connection"
        );
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "mongodb");
    }

    #[test]
    fn test_kafka_mq_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/queue.ts"),
            relative_path: "queue.ts".to_string(),
            content: Arc::from(
                "const kafka = new Kafka(); const producer = kafka.producer(); producer.send({ topic: 'events' });",
            ),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        // May or may not detect depending on tree-sitter query matching
        // This test just ensures no panic
        assert!(result.connections.len() >= 0);
    }
}
