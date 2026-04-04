// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::sync::OnceLock;

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

use crate::plugin::{scope_to_service, ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, EndpointInfo, ExtractionResult};

/// Rust language plugin.
/// Covers .rs and Cargo.toml.
/// Detects: Actix-web routes, Axum Router routes, reqwest/tonic clients, tokio-modbus industrial
/// Note: Module named rust_lang to avoid keyword conflict.
pub struct RustLangPlugin;

/// Framework detection state
#[derive(Debug, Clone, Default)]
struct FrameworkSet {
    actix: bool,
    axum: bool,
    rocket: bool,
    reqwest: bool,
    tonic: bool,
    tokio_modbus: bool,
}

// Actix-web route detection query
const ACTIX_ROUTE_QUERY: &str = r#"
(attribute_item
  (attribute
    (identifier) @macro_name
    (token_tree
      (string_literal
        (string_content) @path))))
"#;

// Axum Router::new().route() detection query
const AXUM_ROUTE_QUERY: &str = r#"
(call_expression
  function: (field_expression
    value: (call_expression) @router_chain
    field: (field_identifier) @method)
  arguments: (arguments
    (string_literal) @path
    (_) @handler))
"#;

// reqwest client detection query
const REQWEST_CLIENT_QUERY: &str = r#"
(call_expression
  (scoped_identifier
    (identifier) @crate
    (identifier) @method)
  (arguments
    (string_literal
      (string_content) @url)))
"#;

// tonic gRPC client detection query
const TONIC_CLIENT_QUERY: &str = r#"
(call_expression
  function: (scoped_identifier
    path: (identifier) @service_name
    name: (identifier) @method)
  arguments: (arguments (_) @url (_)*))
"#;

// tokio-modbus connection query
const TOKIO_MODBUS_QUERY: &str = r#"
(call_expression
  function: (scoped_identifier
    path: (identifier) @modbus_pkg
    name: (identifier) @method)
  arguments: (arguments (_) @addr (_)*))
"#;

// OnceLock query caches
static ACTIX_QUERY: OnceLock<Query> = OnceLock::new();
static AXUM_QUERY: OnceLock<Query> = OnceLock::new();
static REQWEST_QUERY: OnceLock<Query> = OnceLock::new();
static TONIC_QUERY: OnceLock<Query> = OnceLock::new();
static TOKIO_MODBUS_QUERY_CACHE: OnceLock<Query> = OnceLock::new();

fn actix_query(lang: &Language) -> &'static Query {
    ACTIX_QUERY.get_or_init(|| Query::new(lang, ACTIX_ROUTE_QUERY).expect("valid actix query"))
}

fn axum_query(lang: &Language) -> &'static Query {
    AXUM_QUERY.get_or_init(|| Query::new(lang, AXUM_ROUTE_QUERY).expect("valid axum query"))
}

fn reqwest_query(lang: &Language) -> &'static Query {
    REQWEST_QUERY
        .get_or_init(|| Query::new(lang, REQWEST_CLIENT_QUERY).expect("valid reqwest query"))
}

fn tonic_query(lang: &Language) -> &'static Query {
    TONIC_QUERY.get_or_init(|| Query::new(lang, TONIC_CLIENT_QUERY).expect("valid tonic query"))
}

fn tokio_modbus_query(lang: &Language) -> &'static Query {
    TOKIO_MODBUS_QUERY_CACHE
        .get_or_init(|| Query::new(lang, TOKIO_MODBUS_QUERY).expect("valid tokio-modbus query"))
}

/// Detect Rust frameworks from Cargo.toml in the file list
fn detect_frameworks(ctx: &ExtractionContext) -> FrameworkSet {
    let mut frameworks = FrameworkSet::default();

    for file in &ctx.files {
        if file.relative_path.ends_with("Cargo.toml") {
            let content = &*file.content;
            if content.contains("actix-web") {
                frameworks.actix = true;
            }
            if content.contains("axum") {
                frameworks.axum = true;
            }
            if content.contains("rocket") {
                frameworks.rocket = true;
            }
            if content.contains("reqwest") {
                frameworks.reqwest = true;
            }
            if content.contains("tonic") {
                frameworks.tonic = true;
            }
            if content.contains("tokio-modbus") || content.contains("tokio_modbus") {
                frameworks.tokio_modbus = true;
            }
        }
    }

    frameworks
}

/// Extract the HTTP method from Actix macro name
fn actix_method_to_http(macro_name: &str) -> String {
    match macro_name.to_lowercase().as_str() {
        "get" => "GET".to_string(),
        "post" => "POST".to_string(),
        "put" => "PUT".to_string(),
        "delete" => "DELETE".to_string(),
        "patch" => "PATCH".to_string(),
        "head" => "HEAD".to_string(),
        "options" => "OPTIONS".to_string(),
        _ => macro_name.to_uppercase(),
    }
}

/// Extract HTTP method from Axum handler argument (e.g., "get(" -> "GET")
fn axum_handler_to_method(handler: &str) -> String {
    let handler_lower = handler.to_lowercase();
    if handler_lower.contains("get(") {
        "GET".to_string()
    } else if handler_lower.contains("post(") {
        "POST".to_string()
    } else if handler_lower.contains("put(") {
        "PUT".to_string()
    } else if handler_lower.contains("delete(") {
        "DELETE".to_string()
    } else if handler_lower.contains("patch(") {
        "PATCH".to_string()
    } else if handler_lower.contains("head(") {
        "HEAD".to_string()
    } else if handler_lower.contains("options(") {
        "OPTIONS".to_string()
    } else {
        "GET".to_string()
    }
}

/// Format evidence string, capping at 200 chars
fn format_evidence(text: &str) -> String {
    if text.len() > 200 {
        format!("{}...", &text[..197])
    } else {
        text.to_string()
    }
}

/// Extract source_file location as "relative_path:line"
fn format_source_file(relative_path: &str, line: usize) -> String {
    format!("{}:{}", relative_path, line)
}

impl LanguagePlugin for RustLangPlugin {
    fn name(&self) -> &str {
        "rust"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.rs", "**/Cargo.toml"]
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let frameworks = detect_frameworks(ctx);

        // If no Rust frameworks detected, return empty result
        if !frameworks.actix
            && !frameworks.axum
            && !frameworks.rocket
            && !frameworks.reqwest
            && !frameworks.tonic
            && !frameworks.tokio_modbus
        {
            return ExtractionResult::default();
        }

        let lang: Language = tree_sitter_rust::LANGUAGE.into();
        let mut parser = Parser::new();
        let _ = parser.set_language(&lang);

        let mut endpoints = Vec::new();
        let mut connections = Vec::new();

        // Process .rs files only (not Cargo.toml)
        for file in &ctx.files {
            if !file.relative_path.ends_with(".rs") {
                continue;
            }

            let source = &*file.content;
            let file_bytes = source.as_bytes();

            // Parse once per file
            let tree = match parser.parse(file_bytes, None) {
                Some(t) => t,
                None => continue,
            };

            let source_service = scope_to_service(&file.path, &ctx.service_roots);

            // Detect Actix routes
            if frameworks.actix {
                let query = actix_query(&lang);
                let mut cursor = QueryCursor::new();
                let mut matches = cursor.matches(query, tree.root_node(), file_bytes);

                while let Some(m) = matches.next() {
                    let mut macro_name = String::new();
                    let mut path = String::new();
                    let _line = 0;

                    for capture in m.captures {
                        let cap_name = query.capture_names()[capture.index as usize];
                        let text = capture
                            .node
                            .utf8_text(file_bytes)
                            .unwrap_or("")
                            .trim_matches('"');

                        match cap_name {
                            "macro_name" => macro_name = text.to_string(),
                            "path" => path = text.to_string(),
                            _ => {}
                        }
                    }

                    if !macro_name.is_empty()
                        && !path.is_empty()
                        && ["get", "post", "put", "delete", "patch", "head", "options"]
                            .contains(&macro_name.as_str())
                    {
                        endpoints.push(EndpointInfo {
                            service_name: source_service.unwrap_or("").to_string(),
                            method: actix_method_to_http(&macro_name),
                            path: path.clone(),
                            handler: None,
                            kind: "rest".to_string(),
                            confidence: Confidence::High,
                            extraction_method: "ast_actix_macro".to_string(),
                        });
                    }
                }
            }

            // Detect Axum routes
            if frameworks.axum {
                let query = axum_query(&lang);
                let mut cursor = QueryCursor::new();
                let mut matches = cursor.matches(query, tree.root_node(), file_bytes);

                while let Some(m) = matches.next() {
                    let mut path = String::new();
                    let mut handler = String::new();
                    let _line = 0;

                    for capture in m.captures {
                        let cap_name = query.capture_names()[capture.index as usize];
                        let text = capture
                            .node
                            .utf8_text(file_bytes)
                            .unwrap_or("")
                            .trim_matches('"');

                        match cap_name {
                            "path" => path = text.to_string(),
                            "handler" => handler = text.to_string(),
                            _ => {}
                        }
                    }

                    if !path.is_empty() {
                        let method = axum_handler_to_method(&handler);
                        endpoints.push(EndpointInfo {
                            service_name: source_service.unwrap_or("").to_string(),
                            method,
                            path,
                            handler: Some(handler),
                            kind: "rest".to_string(),
                            confidence: Confidence::High,
                            extraction_method: "ast_axum_route".to_string(),
                        });
                    }
                }
            }

            // Detect reqwest clients
            let query = reqwest_query(&lang);
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(query, tree.root_node(), file_bytes);

            while let Some(m) = matches.next() {
                let mut crate_name = String::new();
                let mut method = String::new();
                let mut url = String::new();
                let mut line = 0;
                let mut evidence = String::new();

                for capture in m.captures {
                    let cap_name = query.capture_names()[capture.index as usize];
                    let text = capture
                        .node
                        .utf8_text(file_bytes)
                        .unwrap_or("")
                        .trim_matches('"');
                    line = capture.node.start_position().row + 1;
                    evidence = format_evidence(text);

                    match cap_name {
                        "crate" => crate_name = text.to_string(),
                        "method" => method = text.to_string(),
                        "url" => url = text.to_string(),
                        _ => {}
                    }
                }

                if crate_name == "reqwest"
                    || method.to_lowercase().as_str().contains("get")
                    || method.to_lowercase().as_str().contains("post")
                    || method.to_lowercase().as_str().contains("put")
                {
                    connections.push(ConnectionInfo {
                        source_service: source_service.unwrap_or("").to_string(),
                        target_name: String::new(),
                        protocol: "rest".to_string(),
                        method: Some(method),
                        path: Some(url),
                        source_file: format_source_file(&file.relative_path, line),
                        confidence: Confidence::High,
                        extraction_method: "ast_reqwest_client".to_string(),
                        evidence: Some(evidence),
                    });
                }
            }

            // Detect tonic gRPC clients (gate on tonic in Cargo.toml)
            if frameworks.tonic {
                let query = tonic_query(&lang);
                let mut cursor = QueryCursor::new();
                let mut matches = cursor.matches(query, tree.root_node(), file_bytes);

                while let Some(m) = matches.next() {
                    let mut service_name = String::new();
                    let mut method = String::new();
                    let mut url = String::new();
                    let mut line = 0;
                    let mut evidence = String::new();

                    for capture in m.captures {
                        let cap_name = query.capture_names()[capture.index as usize];
                        let text = capture
                            .node
                            .utf8_text(file_bytes)
                            .unwrap_or("")
                            .trim_matches('"');
                        line = capture.node.start_position().row + 1;
                        evidence = format_evidence(text);

                        match cap_name {
                            "service_name" => service_name = text.to_string(),
                            "method" => method = text.to_string(),
                            "url" => url = text.to_string(),
                            _ => {}
                        }
                    }

                    // Filter for gRPC patterns like "ServiceClient::connect"
                    if method.to_lowercase().contains("connect")
                        && service_name.to_lowercase().contains("client")
                    {
                        connections.push(ConnectionInfo {
                            source_service: source_service.unwrap_or("").to_string(),
                            target_name: service_name.clone(),
                            protocol: "grpc".to_string(),
                            method: Some(method),
                            path: Some(url),
                            source_file: format_source_file(&file.relative_path, line),
                            confidence: Confidence::High,
                            extraction_method: "ast_tonic_client".to_string(),
                            evidence: Some(evidence),
                        });
                    }
                }
            }

            // Detect tokio-modbus industrial protocol (gate on tokio-modbus in Cargo.toml and file content)
            if frameworks.tokio_modbus && source.contains("tokio_modbus") {
                let query = tokio_modbus_query(&lang);
                let mut cursor = QueryCursor::new();
                let mut matches = cursor.matches(query, tree.root_node(), file_bytes);

                while let Some(m) = matches.next() {
                    let mut modbus_pkg = String::new();
                    let mut method = String::new();
                    let mut addr = String::new();
                    let mut line = 0;
                    let mut evidence = String::new();

                    for capture in m.captures {
                        let cap_name = query.capture_names()[capture.index as usize];
                        let text = capture
                            .node
                            .utf8_text(file_bytes)
                            .unwrap_or("")
                            .trim_matches('"');
                        line = capture.node.start_position().row + 1;
                        evidence = format_evidence(text);

                        match cap_name {
                            "modbus_pkg" => modbus_pkg = text.to_string(),
                            "method" => method = text.to_string(),
                            "addr" => addr = text.to_string(),
                            _ => {}
                        }
                    }

                    // Filter for tcp::connect or rtu::connect patterns
                    if (modbus_pkg == "tcp" || modbus_pkg == "rtu") && method == "connect" {
                        connections.push(ConnectionInfo {
                            source_service: source_service.unwrap_or("").to_string(),
                            target_name: format!("{}_{}", modbus_pkg, "device"),
                            protocol: "modbus".to_string(),
                            method: Some(method),
                            path: Some(addr),
                            source_file: format_source_file(&file.relative_path, line),
                            confidence: Confidence::High,
                            extraction_method: "ast_tokio_modbus".to_string(),
                            evidence: Some(evidence),
                        });
                    }
                }
            }
        }

        ExtractionResult {
            services: Vec::new(),
            endpoints,
            connections,
            schemas: Vec::new(),
            actors: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::FileContext;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn create_context(files: Vec<(&str, &str, &str)>) -> ExtractionContext {
        let file_contexts = files
            .into_iter()
            .map(|(path, relative_path, content)| FileContext {
                path: PathBuf::from(path),
                relative_path: relative_path.to_string(),
                content: Arc::from(content),
            })
            .collect();

        ExtractionContext {
            files: file_contexts,
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        }
    }

    #[test]
    fn test_actix_with_cargo_marker() {
        let plugin = RustLangPlugin;

        let files = vec![
            (
                "/repo/Cargo.toml",
                "Cargo.toml",
                r#"[dependencies]
actix-web = "4.0""#,
            ),
            (
                "/repo/src/main.rs",
                "src/main.rs",
                r#"#[get("/users")] async fn list_users() { }"#,
            ),
        ];

        let ctx = create_context(files);
        let result = plugin.extract(&ctx);

        assert!(!result.endpoints.is_empty());
        let ep = &result.endpoints[0];
        assert_eq!(ep.method, "GET");
        assert_eq!(ep.path, "/users");
        assert_eq!(ep.kind, "rest");
        assert_eq!(ep.confidence, Confidence::High);
    }

    #[test]
    fn test_axum_router_detection() {
        let plugin = RustLangPlugin;

        let files = vec![
            (
                "/repo/Cargo.toml",
                "Cargo.toml",
                r#"[dependencies]
axum = "0.7""#,
            ),
            (
                "/repo/src/main.rs",
                "src/main.rs",
                r#"Router::new().route("/orders", get(list_orders))"#,
            ),
        ];

        let ctx = create_context(files);
        let result = plugin.extract(&ctx);

        assert!(!result.endpoints.is_empty());
        let ep = &result.endpoints[0];
        assert_eq!(ep.method, "GET");
        assert_eq!(ep.path, "/orders");
        assert_eq!(ep.extraction_method, "ast_axum_route");
    }

    #[test]
    fn test_reqwest_client_detection() {
        let plugin = RustLangPlugin;

        let files = vec![
            (
                "/repo/Cargo.toml",
                "Cargo.toml",
                r#"[dependencies]
reqwest = "0.11""#,
            ),
            (
                "/repo/src/main.rs",
                "src/main.rs",
                r#"reqwest::get("http://svc/api").await"#,
            ),
        ];

        let ctx = create_context(files);
        let result = plugin.extract(&ctx);

        assert!(
            !result.connections.is_empty(),
            "Expected reqwest connection"
        );
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "rest");
        assert_eq!(conn.extraction_method, "ast_reqwest_client");
    }

    #[test]
    fn test_tokio_modbus_detection() {
        let plugin = RustLangPlugin;

        let files = vec![
            (
                "/repo/Cargo.toml",
                "Cargo.toml",
                r#"[dependencies]
tokio-modbus = "0.1""#,
            ),
            (
                "/repo/src/main.rs",
                "src/main.rs",
                r#"use tokio_modbus::prelude::*;
tcp::connect(addr).await"#,
            ),
        ];

        let ctx = create_context(files);
        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty());
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "modbus");
        assert_eq!(conn.confidence, Confidence::High);
        assert_eq!(conn.extraction_method, "ast_tokio_modbus");
    }

    #[test]
    fn test_source_file_format() {
        assert_eq!(format_source_file("src/main.rs", 42), "src/main.rs:42");
    }

    #[test]
    fn test_evidence_capping() {
        let long_text = "a".repeat(300);
        let capped = format_evidence(&long_text);
        assert!(capped.len() <= 200);
        assert!(capped.ends_with("..."));

        let short_text = "short";
        let result = format_evidence(short_text);
        assert_eq!(result, "short");
    }
}
