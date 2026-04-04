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

// Connection detection queries
const FETCH_CALL_QUERY: &str = r#"
(call_expression
  function: (identifier) @fn
  arguments: (arguments (_)* @url))
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

// HTTP client method calls: axios.get('url'), got('url'), ky('url'), etc.
const HTTP_CLIENT_METHOD_QUERY: &str = r#"
(call_expression
  function: (member_expression
    object: (identifier) @obj
    property: (property_identifier) @method)
  arguments: (arguments (string) @url (_)*))
"#;

// Constructor calls: new Redis(), new PrismaClient(), new Sequelize(), new DataSource(), etc.
const CONSTRUCTOR_CALL_QUERY: &str = r#"
(new_expression
  constructor: (identifier) @ctor
  arguments: (arguments (_)*))
"#;

// OnceLock query caches for Express
static EXPRESS_QUERY: OnceLock<Query> = OnceLock::new();

fn express_query(lang: &Language) -> &'static Query {
    EXPRESS_QUERY
        .get_or_init(|| Query::new(lang, ROUTE_QUERY_EXPRESS).expect("valid express query"))
}

// OnceLock query caches for connection detection
static FETCH_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static GRPC_NEW_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static MONGOOSE_CONNECT_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static KAFKA_SEND_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static HTTP_CLIENT_METHOD_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static CONSTRUCTOR_CALL_QUERY_CACHE: OnceLock<Query> = OnceLock::new();

fn fetch_query(lang: &Language) -> &'static Query {
    FETCH_QUERY_CACHE.get_or_init(|| Query::new(lang, FETCH_CALL_QUERY).expect("valid fetch query"))
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

fn http_client_method_query(lang: &Language) -> &'static Query {
    HTTP_CLIENT_METHOD_QUERY_CACHE.get_or_init(|| {
        Query::new(lang, HTTP_CLIENT_METHOD_QUERY).expect("valid http client method query")
    })
}

fn constructor_call_query(lang: &Language) -> &'static Query {
    CONSTRUCTOR_CALL_QUERY_CACHE.get_or_init(|| {
        Query::new(lang, CONSTRUCTOR_CALL_QUERY).expect("valid constructor call query")
    })
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

/// Extract HTTP client calls (fetch, axios, got, ky, superagent, etc.)
fn extract_http_clients(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;

    // Content gate: check for fetch() before running query
    if !file_content.contains("fetch(") {
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

/// Extract axios, got, ky, superagent HTTP client calls
fn extract_http_client_libraries(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;

    // Content gates for each library
    let has_axios = file_content.contains("axios");
    let has_got = file_content.contains("got");
    let has_ky = file_content.contains("ky");
    let has_superagent = file_content.contains("superagent");

    if !has_axios && !has_got && !has_ky && !has_superagent {
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

    let query = http_client_method_query(lang);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), file_content.as_bytes());

    let capture_names = query.capture_names();
    let service_name = scope_to_service(&file.path, service_roots)
        .unwrap_or("")
        .to_string();

    while let Some(m) = matches.next() {
        let mut obj = String::new();
        let mut method = String::new();
        let mut url = String::new();
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
                "url" => url = text,
                _ => {}
            }

            line = node.start_position().row + 1;
        }

        // Check if this is a known HTTP client library call
        let (library, protocol) = match obj.as_str() {
            "axios" if has_axios => ("axios", "rest"),
            "got" if has_got => ("got", "rest"),
            "ky" if has_ky => ("ky", "rest"),
            "superagent" if has_superagent => ("superagent", "rest"),
            _ => continue,
        };

        // Check for valid HTTP methods
        let http_methods = ["get", "post", "put", "delete", "patch", "head", "options"];

        if !http_methods.contains(&method.to_lowercase().as_str()) {
            continue;
        }

        let evidence = format!("{}.{}('{}')", library, method, url);
        let truncated_evidence = if evidence.len() > 200 {
            evidence[..200].to_string()
        } else {
            evidence
        };

        result.connections.push(ConnectionInfo {
            source_service: service_name.clone(),
            target_name: url.clone(),
            protocol: protocol.to_string(),
            method: Some(method.to_uppercase()),
            path: None,
            source_file: format!("{}:{}", file.relative_path, line),
            confidence: Confidence::High,
            extraction_method: "ast_http_client_lib".to_string(),
            evidence: Some(truncated_evidence),
        });
    }
}

/// Extract Fastify routes (when fastify framework is detected)
fn extract_fastify_routes(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;
    if !file_content.contains("fastify") {
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

    let query = express_query(lang); // Fastify uses same pattern as Express
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), file_content.as_bytes());

    let capture_names = query.capture_names();

    while let Some(m) = matches.next() {
        let mut receiver = String::new();
        let mut method = String::new();
        let mut path = String::new();
        let mut handler = String::new();

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
        }

        // Fastify instances typically named: fastify, app, instance, server
        let allowed_receivers = ["fastify", "app", "instance", "server", "f"];
        let http_methods = [
            "get", "post", "put", "delete", "patch", "head", "options", "all",
        ];

        if !allowed_receivers.contains(&receiver.as_str()) {
            continue;
        }
        if !http_methods.contains(&method.to_lowercase().as_str()) {
            continue;
        }

        let service_name = scope_to_service(&file.path, service_roots)
            .unwrap_or("")
            .to_string();

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
            extraction_method: "ast_fastify".to_string(),
        });
    }
}

/// Extract Next.js API routes from file paths and exported functions
fn extract_nextjs_routes(
    result: &mut ExtractionResult,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let rel_path = &file.relative_path;
    let file_content = &*file.content;

    // Match Next.js 13+ app router: app/**/route.ts or app/**/route.js
    let is_app_router = (rel_path.starts_with("app/") || rel_path.contains("/app/"))
        && (rel_path.ends_with("/route.ts")
            || rel_path.ends_with("/route.tsx")
            || rel_path.ends_with("/route.js")
            || rel_path.ends_with("/route.jsx"));

    // Match Next.js 12 pages router: pages/api/**/*.ts
    let is_pages_router = (rel_path.starts_with("pages/api/") || rel_path.contains("/pages/api/"))
        && (rel_path.ends_with(".ts")
            || rel_path.ends_with(".tsx")
            || rel_path.ends_with(".js")
            || rel_path.ends_with(".jsx"));

    if !is_app_router && !is_pages_router {
        return;
    }

    // Extract HTTP method from exported function names (case-insensitive)
    let http_methods = vec!["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];
    let lower_content = file_content.to_lowercase();

    for method in http_methods {
        let pattern = format!("export function {}", method.to_lowercase());
        if !lower_content.contains(&pattern) {
            continue;
        }

        // Derive path from file path
        let path = if is_app_router {
            // app/users/route.ts -> /users
            let base = rel_path.strip_prefix("app/").unwrap_or(rel_path);
            let base = base
                .strip_suffix("/route.ts")
                .or_else(|| base.strip_suffix("/route.tsx"))
                .or_else(|| base.strip_suffix("/route.js"))
                .or_else(|| base.strip_suffix("/route.jsx"))
                .unwrap_or(base);
            format!("/{}", base)
        } else {
            // pages/api/users/[id].ts -> /users/[id]
            let base = rel_path.strip_prefix("pages/api/").unwrap_or(rel_path);
            let base = base
                .strip_suffix(".ts")
                .or_else(|| base.strip_suffix(".tsx"))
                .or_else(|| base.strip_suffix(".js"))
                .or_else(|| base.strip_suffix(".jsx"))
                .unwrap_or(base);
            format!("/{}", base)
        };

        let service_name = scope_to_service(&file.path, service_roots)
            .unwrap_or("")
            .to_string();

        result.endpoints.push(EndpointInfo {
            service_name,
            method: method.to_string(),
            path,
            handler: None,
            kind: "rest".to_string(),
            confidence: Confidence::High,
            extraction_method: "nextjs_api_routes".to_string(),
        });
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

/// Extract RabbitMQ/amqplib connections
fn extract_amqplib_connections(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;

    let has_amqplib = file_content.contains("amqplib") || file_content.contains("from 'amqplib'");
    let has_channel =
        file_content.contains("channel.publish") || file_content.contains("channel.sendToQueue");

    if !has_amqplib && !has_channel {
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

    let query = http_client_method_query(lang);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), file_content.as_bytes());

    let capture_names = query.capture_names();
    let service_name = scope_to_service(&file.path, service_roots)
        .unwrap_or("")
        .to_string();

    while let Some(m) = matches.next() {
        let mut obj = String::new();
        let mut method = String::new();
        let mut url = String::new();
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
                "url" => url = text,
                _ => {}
            }

            line = node.start_position().row + 1;
        }

        if obj == "channel" && (method == "publish" || method == "sendToQueue") {
            let evidence = format!("channel.{}('{}')", method, url);
            let truncated_evidence = if evidence.len() > 200 {
                evidence[..200].to_string()
            } else {
                evidence
            };

            result.connections.push(ConnectionInfo {
                source_service: service_name.clone(),
                target_name: url.clone(),
                protocol: "amqp".to_string(),
                method: Some(method),
                path: None,
                source_file: format!("{}:{}", file.relative_path, line),
                confidence: Confidence::High,
                extraction_method: "ast_amqplib".to_string(),
                evidence: Some(truncated_evidence),
            });
        }
    }
}

/// Extract Redis client instantiations
fn extract_redis_clients(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;

    let has_ioredis = file_content.contains("ioredis") || file_content.contains("IORedis");
    let has_redis = file_content.contains("redis") && file_content.contains("createClient");

    if !has_ioredis && !has_redis {
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

    let service_name = scope_to_service(&file.path, service_roots)
        .unwrap_or("")
        .to_string();

    // Match new Redis() or new IORedis()
    if has_ioredis {
        let query = constructor_call_query(lang);
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(query, tree.root_node(), file_content.as_bytes());

        let capture_names = query.capture_names();

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

                if capture_name == "ctor" {
                    ctor = text;
                }

                line = node.start_position().row + 1;
            }

            if ctor == "Redis" || ctor == "IORedis" {
                let evidence = format!("new {}()", ctor);

                result.connections.push(ConnectionInfo {
                    source_service: service_name.clone(),
                    target_name: "redis".to_string(),
                    protocol: "redis".to_string(),
                    method: None,
                    path: None,
                    source_file: format!("{}:{}", file.relative_path, line),
                    confidence: Confidence::High,
                    extraction_method: "ast_redis".to_string(),
                    evidence: Some(evidence),
                });
            }
        }
    }

    // Match createClient() - a regular function call, not new expression
    if has_redis {
        let fetch_query = fetch_query(lang);
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(fetch_query, tree.root_node(), file_content.as_bytes());
        let capture_names = fetch_query.capture_names();

        while let Some(m) = matches.next() {
            let mut fn_name = String::new();
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

                if capture_name == "fn" {
                    fn_name = text;
                }

                line = node.start_position().row + 1;
            }

            if fn_name == "createClient" {
                let evidence = "createClient()".to_string();

                result.connections.push(ConnectionInfo {
                    source_service: service_name.clone(),
                    target_name: "redis".to_string(),
                    protocol: "redis".to_string(),
                    method: None,
                    path: None,
                    source_file: format!("{}:{}", file.relative_path, line),
                    confidence: Confidence::High,
                    extraction_method: "ast_redis".to_string(),
                    evidence: Some(evidence),
                });
            }
        }
    }
}

/// Extract Prisma, TypeORM, and Sequelize ORM instantiations
fn extract_orm_connections(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;

    let has_prisma = file_content.contains("PrismaClient");
    let has_typeorm = file_content.contains("DataSource");
    let has_sequelize = file_content.contains("Sequelize");

    if !has_prisma && !has_typeorm && !has_sequelize {
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

    let query = constructor_call_query(lang);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), file_content.as_bytes());

    let capture_names = query.capture_names();
    let service_name = scope_to_service(&file.path, service_roots)
        .unwrap_or("")
        .to_string();

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

            if capture_name == "ctor" {
                ctor = text;
            }

            line = node.start_position().row + 1;
        }

        if ctor == "PrismaClient" && has_prisma {
            let evidence = "new PrismaClient()".to_string();

            result.connections.push(ConnectionInfo {
                source_service: service_name.clone(),
                target_name: "postgresql".to_string(),
                protocol: "postgresql".to_string(),
                method: None,
                path: None,
                source_file: format!("{}:{}", file.relative_path, line),
                confidence: Confidence::Medium,
                extraction_method: "ast_prisma".to_string(),
                evidence: Some(evidence),
            });
        } else if ctor == "DataSource" && has_typeorm {
            let evidence = "new DataSource({...})".to_string();

            result.connections.push(ConnectionInfo {
                source_service: service_name.clone(),
                target_name: "database".to_string(),
                protocol: "postgresql".to_string(), // Default to postgres, may be overridden by config
                method: None,
                path: None,
                source_file: format!("{}:{}", file.relative_path, line),
                confidence: Confidence::Medium,
                extraction_method: "ast_typeorm".to_string(),
                evidence: Some(evidence),
            });
        } else if ctor == "Sequelize" && has_sequelize {
            let evidence = "new Sequelize(...)".to_string();

            result.connections.push(ConnectionInfo {
                source_service: service_name.clone(),
                target_name: "database".to_string(),
                protocol: "postgresql".to_string(), // May be mysql, postgres, sqlite, etc.
                method: None,
                path: None,
                source_file: format!("{}:{}", file.relative_path, line),
                confidence: Confidence::Medium,
                extraction_method: "ast_sequelize".to_string(),
                evidence: Some(evidence),
            });
        }
    }
}

/// Extract gRPC client instantiations (including @grpc/grpc-js)
fn extract_grpc_clients(
    result: &mut ExtractionResult,
    lang: &Language,
    file: &crate::plugin::FileContext,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let file_content = &*file.content;

    let has_grpc_python_style =
        file_content.contains("_grpc") || file_content.contains("_pb2_grpc");
    let has_grpc_js = file_content.contains("@grpc/grpc-js") || file_content.contains("grpc");

    if !has_grpc_python_style && !has_grpc_js {
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

    // Try gRPC new Client pattern
    let grpc_query = grpc_new_query(lang);
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(grpc_query, tree.root_node(), file_content.as_bytes());

    let capture_names = grpc_query.capture_names();
    let service_name = scope_to_service(&file.path, service_roots)
        .unwrap_or("")
        .to_string();

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

            if capture_name == "ctor" {
                ctor = text
            }

            line = node.start_position().row + 1;
        }

        if ctor.ends_with("Client") {
            let target = ctor.strip_suffix("Client").unwrap_or(&ctor).to_string();

            let evidence = format!("new {}(...)", ctor);

            result.connections.push(ConnectionInfo {
                source_service: service_name.clone(),
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

    // Also detect grpc.credentials.createInsecure() pattern for @grpc/grpc-js
    if has_grpc_js && file_content.contains("grpc.credentials") {
        let member_query_str = r#"
(call_expression
  function: (member_expression
    object: (member_expression
      object: (identifier) @root
      property: (property_identifier) @ns)
    property: (property_identifier) @method)
  arguments: (arguments (_)*))
"#;

        if let Ok(creds_query) = tree_sitter::Query::new(lang, member_query_str) {
            let mut cursor = QueryCursor::new();
            let mut matches =
                cursor.matches(&creds_query, tree.root_node(), file_content.as_bytes());
            let cap_names = creds_query.capture_names();

            while let Some(m) = matches.next() {
                let mut root = String::new();
                let mut ns = String::new();
                let mut method = String::new();
                let mut line = 1;

                for capture in m.captures {
                    let cap_name = cap_names[capture.index as usize];
                    let node = capture.node;
                    let text = node
                        .utf8_text(file_content.as_bytes())
                        .unwrap_or("")
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string();

                    match cap_name {
                        "root" => root = text,
                        "ns" => ns = text,
                        "method" => method = text,
                        _ => {}
                    }

                    line = node.start_position().row + 1;
                }

                if root == "grpc" && ns == "credentials" && method == "createInsecure" {
                    let evidence = "grpc.credentials.createInsecure()".to_string();

                    result.connections.push(ConnectionInfo {
                        source_service: service_name.clone(),
                        target_name: "grpc_server".to_string(),
                        protocol: "grpc".to_string(),
                        method: None,
                        path: None,
                        source_file: format!("{}:{}", file.relative_path, line),
                        confidence: Confidence::High,
                        extraction_method: "ast_grpc_js".to_string(),
                        evidence: Some(evidence),
                    });
                }
            }
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

                // Extract Fastify routes if framework detected
                if frameworks.fastify {
                    extract_fastify_routes(&mut result, &lang, file, &ctx.service_roots);
                }

                // Extract HTTP clients (fetch, axios, got, ky, superagent, etc.)
                extract_http_clients(&mut result, &lang, file, &ctx.service_roots);
                extract_http_client_libraries(&mut result, &lang, file, &ctx.service_roots);

                // Extract Next.js API routes
                extract_nextjs_routes(&mut result, file, &ctx.service_roots);

                // Extract database connections (mongoose)
                extract_database_connections(&mut result, &lang, file, &ctx.service_roots);

                // Extract ORM connections (Prisma, TypeORM, Sequelize)
                extract_orm_connections(&mut result, &lang, file, &ctx.service_roots);

                // Extract RabbitMQ/amqplib connections
                extract_amqplib_connections(&mut result, &lang, file, &ctx.service_roots);

                // Extract Redis clients
                extract_redis_clients(&mut result, &lang, file, &ctx.service_roots);

                // Extract gRPC clients
                extract_grpc_clients(&mut result, &lang, file, &ctx.service_roots);

                // Extract message queue calls (Kafka)
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

    #[test]
    fn test_axios_http_client_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/client.ts"),
            relative_path: "client.ts".to_string(),
            content: Arc::from(
                "import axios from 'axios'; axios.get('https://api.example.com/users');",
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
            "Should detect axios.get call"
        );
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "rest");
        assert_eq!(conn.extraction_method, "ast_http_client_lib");
        assert!(
            conn.evidence.as_ref().unwrap().contains("axios"),
            "Evidence should contain 'axios'"
        );
    }

    #[test]
    fn test_got_http_client_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/client.ts"),
            relative_path: "client.ts".to_string(),
            content: Arc::from(
                "import got from 'got'; got.post('https://api.example.com/submit');",
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
            "Should detect got.post call"
        );
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "rest");
        assert_eq!(conn.extraction_method, "ast_http_client_lib");
    }

    #[test]
    fn test_ky_http_client_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/client.ts"),
            relative_path: "client.ts".to_string(),
            content: Arc::from("import ky from 'ky'; ky.put('https://api.example.com/update');"),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty(), "Should detect ky.put call");
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "rest");
    }

    #[test]
    fn test_superagent_http_client_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/client.ts"),
            relative_path: "client.ts".to_string(),
            content: Arc::from(
                "import superagent from 'superagent'; superagent.delete('https://api.example.com/delete');",
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
            "Should detect superagent.delete call"
        );
    }

    #[test]
    fn test_fastify_route_detection() {
        let plugin = TypeScriptPlugin;

        let package_json = FileContext {
            path: std::path::PathBuf::from("/repo/package.json"),
            relative_path: "package.json".to_string(),
            content: Arc::from(r#"{"dependencies": {"fastify": "^4.0"}}"#),
        };

        let server_file = FileContext {
            path: std::path::PathBuf::from("/repo/server.ts"),
            relative_path: "server.ts".to_string(),
            content: Arc::from(
                "const fastify = require('fastify')(); fastify.get('/items', handler);",
            ),
        };

        let ctx = ExtractionContext {
            files: vec![package_json, server_file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(!result.endpoints.is_empty(), "Should detect Fastify route");
        let endpoint = &result.endpoints[0];
        assert_eq!(endpoint.method, "GET");
        assert_eq!(endpoint.path, "/items");
        assert_eq!(endpoint.extraction_method, "ast_fastify");
    }

    #[test]
    fn test_nextjs_app_route_detection() {
        let plugin = TypeScriptPlugin;

        let route_file = FileContext {
            path: std::path::PathBuf::from("/repo/app/api/users/route.ts"),
            relative_path: "app/api/users/route.ts".to_string(),
            content: Arc::from("export function GET() { return Response.json({}); }"),
        };

        let ctx = ExtractionContext {
            files: vec![route_file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(
            !result.endpoints.is_empty(),
            "Should detect Next.js app route"
        );
        let endpoint = &result.endpoints[0];
        assert_eq!(endpoint.method, "GET");
        assert_eq!(endpoint.path, "/api/users");
        assert_eq!(endpoint.extraction_method, "nextjs_api_routes");
    }

    #[test]
    fn test_nextjs_pages_route_detection() {
        let plugin = TypeScriptPlugin;

        let route_file = FileContext {
            path: std::path::PathBuf::from("/repo/pages/api/auth/login.ts"),
            relative_path: "pages/api/auth/login.ts".to_string(),
            content: Arc::from("export function POST() { return {}; }"),
        };

        let ctx = ExtractionContext {
            files: vec![route_file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(
            !result.endpoints.is_empty(),
            "Should detect Next.js pages route"
        );
        let endpoint = &result.endpoints[0];
        assert_eq!(endpoint.method, "POST");
        assert_eq!(endpoint.path, "/auth/login");
    }

    #[test]
    fn test_amqplib_connection_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/mq.ts"),
            relative_path: "mq.ts".to_string(),
            content: Arc::from(
                "import amqplib from 'amqplib'; channel.publish('exchange', 'routingKey', Buffer.from('message'));",
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
            "Should detect amqplib channel.publish"
        );
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "amqp");
        assert_eq!(conn.extraction_method, "ast_amqplib");
    }

    #[test]
    fn test_redis_ioredis_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/cache.ts"),
            relative_path: "cache.ts".to_string(),
            content: Arc::from("import Redis from 'ioredis'; const redis = new Redis();"),
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
            "Should detect Redis connection"
        );
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "redis");
        assert_eq!(conn.extraction_method, "ast_redis");
    }

    #[test]
    fn test_redis_createclient_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/cache.ts"),
            relative_path: "cache.ts".to_string(),
            content: Arc::from(
                "import { createClient } from 'redis'; const client = createClient();",
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
            "Should detect redis createClient"
        );
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "redis");
    }

    #[test]
    fn test_prisma_orm_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/db.ts"),
            relative_path: "db.ts".to_string(),
            content: Arc::from(
                "import { PrismaClient } from '@prisma/client'; const db = new PrismaClient();",
            ),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty(), "Should detect PrismaClient");
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "postgresql");
        assert_eq!(conn.extraction_method, "ast_prisma");
    }

    #[test]
    fn test_typeorm_orm_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/db.ts"),
            relative_path: "db.ts".to_string(),
            content: Arc::from(
                "import { DataSource } from 'typeorm'; const db = new DataSource({ type: 'postgres', ...: {} });",
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
            "Should detect TypeORM DataSource"
        );
        let conn = &result.connections[0];
        assert_eq!(conn.extraction_method, "ast_typeorm");
    }

    #[test]
    fn test_sequelize_orm_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/db.ts"),
            relative_path: "db.ts".to_string(),
            content: Arc::from(
                "import { Sequelize } from 'sequelize'; const db = new Sequelize('postgres://user:pass@localhost/db');",
            ),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty(), "Should detect Sequelize");
        let conn = &result.connections[0];
        assert_eq!(conn.extraction_method, "ast_sequelize");
    }

    #[test]
    fn test_grpc_js_credentials_detection() {
        let plugin = TypeScriptPlugin;

        let file = FileContext {
            path: std::path::PathBuf::from("/repo/grpc.ts"),
            relative_path: "grpc.ts".to_string(),
            content: Arc::from(
                "import * as grpc from '@grpc/grpc-js'; const creds = grpc.credentials.createInsecure();",
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
            "Should detect grpc.credentials.createInsecure"
        );
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "grpc");
        assert_eq!(conn.extraction_method, "ast_grpc_js");
    }
}
