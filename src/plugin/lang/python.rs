// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::sync::OnceLock;
use tree_sitter::{Parser, Query, QueryCursor};

use crate::plugin::{scope_to_service, ExtractionContext, FileContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, EndpointInfo, ExtractionResult};

/// Python language plugin.
/// Covers .py, requirements.txt, and pyproject.toml.
pub struct PythonPlugin;

/// Framework detection: which frameworks are present in this codebase.
#[derive(Debug, Clone, Default)]
struct FrameworkSet {
    fastapi: bool,
    django: bool,
    flask: bool,
}

/// Detect frameworks by scanning manifest files for framework markers.
fn detect_frameworks(ctx: &ExtractionContext) -> FrameworkSet {
    let mut frameworks = FrameworkSet::default();

    for file in &ctx.files {
        let is_req = file.relative_path.ends_with("requirements.txt");
        let is_toml = file.relative_path.ends_with("pyproject.toml");

        if is_req || is_toml {
            let content = &*file.content;
            if content.contains("fastapi") {
                frameworks.fastapi = true;
            }
            if content.contains("django") || content.contains("Django") {
                frameworks.django = true;
            }
            if content.contains("flask") || content.contains("Flask") {
                frameworks.flask = true;
            }
        }
    }

    frameworks
}

/// Compile FastAPI/Flask route query once via OnceLock.
fn fastapi_flask_route_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    QUERY.get_or_init(|| {
        let src = r#"
(decorated_definition
  (decorator
    (call
      function: (attribute
        object: (identifier) @obj
        attribute: (identifier) @http_method)
      arguments: (argument_list (string) @path)))
  definition: (function_definition
    name: (identifier) @handler))
"#;
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        Query::new(&lang, src).expect("invalid fastapi_flask query")
    })
}

/// Compile Django urlpatterns route query once via OnceLock.
fn django_urlpatterns_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    QUERY.get_or_init(|| {
        let src = r#"
(assignment
  left: (identifier) @var_name
  right: (list
    (call
      function: (identifier) @path_fn
      arguments: (argument_list
        (string) @route
        (_) @view))))
"#;
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        Query::new(&lang, src).expect("invalid django_urlpatterns query")
    })
}

/// Compile HTTP client query once via OnceLock.
fn http_client_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    QUERY.get_or_init(|| {
        let src = r#"
(call
  function: (attribute
    object: (identifier) @lib
    attribute: (identifier) @method)
  arguments: (argument_list (string) @url (_)*))
"#;
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        Query::new(&lang, src).expect("invalid http_client query")
    })
}

/// Compile database client query once via OnceLock.
fn db_client_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    QUERY.get_or_init(|| {
        let src = r#"
(call
  function: (attribute
    object: (identifier) @lib
    attribute: (identifier) @method)
  arguments: (argument_list (string) @dsn (_)*))
"#;
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        Query::new(&lang, src).expect("invalid db_client query")
    })
}

/// Compile gRPC stub instantiation query once via OnceLock.
fn grpc_stub_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    QUERY.get_or_init(|| {
        let src = r#"
(call
  function: (identifier) @stub
  arguments: (argument_list (_)))
"#;
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        Query::new(&lang, src).expect("invalid grpc_stub query")
    })
}

impl LanguagePlugin for PythonPlugin {
    fn name(&self) -> &str {
        "python"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.py", "**/requirements.txt", "**/pyproject.toml"]
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        // Step 1: Detect frameworks
        let frameworks = detect_frameworks(ctx);

        let mut result = ExtractionResult::default();

        // Separate manifest files from source files
        let mut source_files = Vec::new();
        for file in &ctx.files {
            if file.relative_path.ends_with(".py") {
                source_files.push(file);
            }
        }

        // Step 2-4: Build parser and extract routes if frameworks detected
        if !source_files.is_empty() {
            let mut parser = Parser::new();
            let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
            if parser.set_language(&lang).is_err() {
                tracing::error!("Failed to set Python parser language");
                return result;
            }

            // Route detection (only if framework markers found)
            if frameworks.fastapi || frameworks.flask {
                let query = fastapi_flask_route_query();
                result.endpoints.extend(extract_fastapi_flask_routes(
                    ctx,
                    &source_files,
                    &mut parser,
                    query,
                ));
            }

            if frameworks.django {
                let query = django_urlpatterns_query();
                result.endpoints.extend(extract_django_routes(
                    ctx,
                    &source_files,
                    &mut parser,
                    query,
                ));
            }

            // Step 5: Client detection (runs regardless of framework)
            result
                .connections
                .extend(extract_http_clients(ctx, &source_files, &mut parser));
            result
                .connections
                .extend(extract_db_clients(ctx, &source_files, &mut parser));
            result
                .connections
                .extend(extract_mq_clients(ctx, &source_files, &mut parser));
            result
                .connections
                .extend(extract_industrial_protocol_clients(
                    ctx,
                    &source_files,
                    &mut parser,
                ));
            result
                .connections
                .extend(extract_grpc_clients(ctx, &source_files, &mut parser));
        }

        result
    }
}

fn extract_fastapi_flask_routes(
    ctx: &ExtractionContext,
    source_files: &[&FileContext],
    parser: &mut Parser,
    query: &Query,
) -> Vec<EndpointInfo> {
    let mut endpoints = Vec::new();
    let http_methods = ["get", "post", "put", "delete", "patch", "options", "head"];

    for file in source_files {
        let source_bytes = file.content.as_bytes();
        let tree = match parser.parse(source_bytes, None) {
            Some(t) => t,
            None => continue,
        };

        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(query, tree.root_node(), source_bytes);

        // Use the StreamingIterator pattern
        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(query, tree.root_node(), source_bytes);
        while let Some(m) = matches.next() {
            let mut obj_name = "";
            let mut http_method = "";
            let mut path = "";
            let mut handler = "";

            for capture in m.captures {
                let name = query.capture_names()[capture.index as usize];
                let text = capture
                    .node
                    .utf8_text(source_bytes)
                    .unwrap_or("")
                    .trim_matches('"')
                    .trim_matches('\'');

                match name {
                    "obj" => obj_name = text,
                    "http_method" => http_method = text,
                    "path" => path = text,
                    "handler" => handler = text,
                    _ => {}
                }
            }

            // After processing all captures in a match, create endpoint if valid
            if !http_method.is_empty()
                && !path.is_empty()
                && !handler.is_empty()
                && http_methods.contains(&http_method)
            {
                let source_service = scope_to_service(&file.path, &ctx.service_roots);
                if source_service.is_none() {
                    tracing::warn!("Unscoped Python file: {}", file.relative_path);
                }

                endpoints.push(EndpointInfo {
                    service_name: source_service.unwrap_or("").to_string(),
                    method: http_method.to_uppercase(),
                    path: path.to_string(),
                    handler: Some(handler.to_string()),
                    kind: "rest".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "python_fastapi_flask_decorator".to_string(),
                });
            }
        }
    }

    endpoints
}

fn extract_django_routes(
    ctx: &ExtractionContext,
    source_files: &[&FileContext],
    parser: &mut Parser,
    query: &Query,
) -> Vec<EndpointInfo> {
    let mut endpoints = Vec::new();
    let path_fns = ["path", "re_path", "url"];

    for file in source_files {
        let source_bytes = file.content.as_bytes();
        let tree = match parser.parse(source_bytes, None) {
            Some(t) => t,
            None => continue,
        };

        let mut cursor = QueryCursor::new();
        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(query, tree.root_node(), source_bytes);

        while let Some(m) = matches.next() {
            let mut var_name = "";
            let mut path_fn = "";
            let mut route = "";

            for capture in m.captures {
                let name = query.capture_names()[capture.index as usize];
                let text = capture
                    .node
                    .utf8_text(source_bytes)
                    .unwrap_or("")
                    .trim_matches('"')
                    .trim_matches('\'');

                match name {
                    "var_name" => var_name = text,
                    "path_fn" => path_fn = text,
                    "route" => route = text,
                    _ => {}
                }
            }

            // Only emit if this is a urlpatterns assignment
            if var_name == "urlpatterns" && path_fns.contains(&path_fn) && !route.is_empty() {
                let source_service = scope_to_service(&file.path, &ctx.service_roots);
                if source_service.is_none() {
                    tracing::warn!("Unscoped Python file: {}", file.relative_path);
                }

                endpoints.push(EndpointInfo {
                    service_name: source_service.unwrap_or("").to_string(),
                    method: "GET".to_string(),
                    path: route.to_string(),
                    handler: None,
                    kind: "rest".to_string(),
                    confidence: Confidence::Medium,
                    extraction_method: "python_django_urlpatterns".to_string(),
                });
            }
        }
    }

    endpoints
}

fn extract_http_clients(
    ctx: &ExtractionContext,
    source_files: &[&FileContext],
    parser: &mut Parser,
) -> Vec<ConnectionInfo> {
    let mut connections = Vec::new();
    let http_libs = ["requests", "httpx", "aiohttp", "urllib"];
    let http_methods = ["get", "post", "put", "delete", "patch"];
    let query = http_client_query();

    for file in source_files {
        let source_bytes = file.content.as_bytes();
        let content = &*file.content;

        // Skip if no HTTP library import
        if !http_libs.iter().any(|lib| content.contains(lib)) {
            continue;
        }

        let tree = match parser.parse(source_bytes, None) {
            Some(t) => t,
            None => continue,
        };

        let mut cursor = QueryCursor::new();
        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(query, tree.root_node(), source_bytes);

        while let Some(m) = matches.next() {
            let mut lib = "";
            let mut method = "";
            let mut url = "";

            for capture in m.captures {
                let name = query.capture_names()[capture.index as usize];
                let text = capture
                    .node
                    .utf8_text(source_bytes)
                    .unwrap_or("")
                    .trim_matches('"')
                    .trim_matches('\'');

                match name {
                    "lib" => lib = text,
                    "method" => method = text,
                    "url" => url = text,
                    _ => {}
                }
            }

            if http_libs.contains(&lib) && http_methods.contains(&method) && !url.is_empty() {
                let source_service = scope_to_service(&file.path, &ctx.service_roots);
                if source_service.is_none() {
                    tracing::warn!("Unscoped Python file: {}", file.relative_path);
                }

                connections.push(ConnectionInfo {
                    source_service: source_service.unwrap_or("").to_string(),
                    target_name: url.to_string(),
                    protocol: "rest".to_string(),
                    method: Some(method.to_uppercase()),
                    path: None,
                    source_file: format!(
                        "{}:{}",
                        file.relative_path,
                        m.captures
                            .first()
                            .map(|c| c.node.start_position().row + 1)
                            .unwrap_or(0)
                    ),
                    confidence: Confidence::High,
                    extraction_method: "python_http_client".to_string(),
                    evidence: Some(url[..std::cmp::min(url.len(), 200)].to_string()),
                });
            }
        }
    }

    connections
}

fn extract_db_clients(
    ctx: &ExtractionContext,
    source_files: &[&FileContext],
    parser: &mut Parser,
) -> Vec<ConnectionInfo> {
    let mut connections = Vec::new();
    let query = db_client_query();

    for file in source_files {
        let source_bytes = file.content.as_bytes();
        let content = &*file.content;

        // Detect which DB libraries are imported
        let (protocol, lib) = if content.contains("asyncpg") {
            ("postgresql", "asyncpg")
        } else if content.contains("psycopg2") {
            ("postgresql", "psycopg2")
        } else if content.contains("motor") {
            ("mongodb", "motor")
        } else if content.contains("redis") {
            ("redis", "redis")
        } else {
            continue;
        };

        let tree = match parser.parse(source_bytes, None) {
            Some(t) => t,
            None => continue,
        };

        let mut cursor = QueryCursor::new();
        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(query, tree.root_node(), source_bytes);

        while let Some(m) = matches.next() {
            let mut lib_name = "";
            let mut dsn = "";

            for capture in m.captures {
                let name = query.capture_names()[capture.index as usize];
                let text = capture
                    .node
                    .utf8_text(source_bytes)
                    .unwrap_or("")
                    .trim_matches('"')
                    .trim_matches('\'');

                match name {
                    "lib" => lib_name = text,
                    "dsn" => dsn = text,
                    _ => {}
                }
            }

            if lib_name == lib && !dsn.is_empty() {
                let source_service = scope_to_service(&file.path, &ctx.service_roots);
                if source_service.is_none() {
                    tracing::warn!("Unscoped Python file: {}", file.relative_path);
                }

                connections.push(ConnectionInfo {
                    source_service: source_service.unwrap_or("").to_string(),
                    target_name: dsn.to_string(),
                    protocol: protocol.to_string(),
                    method: None,
                    path: None,
                    source_file: format!(
                        "{}:{}",
                        file.relative_path,
                        m.captures
                            .first()
                            .map(|c| c.node.start_position().row + 1)
                            .unwrap_or(0)
                    ),
                    confidence: Confidence::High,
                    extraction_method: "python_db_client".to_string(),
                    evidence: Some(dsn[..std::cmp::min(dsn.len(), 200)].to_string()),
                });
            }
        }
    }

    connections
}

fn extract_mq_clients(
    ctx: &ExtractionContext,
    source_files: &[&FileContext],
    _parser: &mut Parser,
) -> Vec<ConnectionInfo> {
    let mut connections = Vec::new();

    for file in source_files {
        let content = &*file.content;

        // Pika: channel.basic_publish(exchange, routing_key, ...)
        if content.contains("pika") && content.contains("basic_publish") {
            let source_service = scope_to_service(&file.path, &ctx.service_roots);
            if source_service.is_none() {
                tracing::warn!("Unscoped Python file: {}", file.relative_path);
            }

            // Extract routing key via simple text pattern (import-gated)
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if line.contains("basic_publish") {
                    if let Some(rest) = line.split("basic_publish").nth(1) {
                        // Simple extraction: look for routing_key quoted
                        if let Some(start) = rest.find('"') {
                            if let Some(end) = rest[start + 1..].find('"') {
                                let routing_key = &rest[start + 1..start + 1 + end];
                                connections.push(ConnectionInfo {
                                    source_service: source_service.unwrap_or("").to_string(),
                                    target_name: "amqp_broker".to_string(),
                                    protocol: "amqp".to_string(),
                                    method: None,
                                    path: Some(routing_key.to_string()),
                                    source_file: format!("{}:{}", file.relative_path, i + 1),
                                    confidence: Confidence::High,
                                    extraction_method: "python_pika_client".to_string(),
                                    evidence: Some(line.to_string()),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    connections
}

fn extract_industrial_protocol_clients(
    ctx: &ExtractionContext,
    source_files: &[&FileContext],
    parser: &mut Parser,
) -> Vec<ConnectionInfo> {
    let mut connections = Vec::new();

    for file in source_files {
        let content = &*file.content;

        // ModbusTcpClient from pymodbus
        if (content.contains("from pymodbus") || content.contains("import pymodbus"))
            && content.contains("ModbusTcpClient")
        {
            let source_service = scope_to_service(&file.path, &ctx.service_roots);
            if source_service.is_none() {
                tracing::warn!("Unscoped Python file: {}", file.relative_path);
            }

            let source_bytes = file.content.as_bytes();
            let tree = match parser.parse(source_bytes, None) {
                Some(t) => t,
                None => continue,
            };

            let stub_query = grpc_stub_query();
            let mut cursor = QueryCursor::new();
            use streaming_iterator::StreamingIterator;
            let mut matches = cursor.matches(stub_query, tree.root_node(), source_bytes);

            while let Some(m) = matches.next() {
                for capture in m.captures {
                    let name = stub_query.capture_names()[capture.index as usize];
                    if name == "stub" {
                        let stub_name = capture
                            .node
                            .utf8_text(source_bytes)
                            .unwrap_or("")
                            .trim_matches('"')
                            .trim_matches('\'');

                        if stub_name == "ModbusTcpClient" {
                            connections.push(ConnectionInfo {
                                source_service: source_service.unwrap_or("").to_string(),
                                target_name: "modbus_device".to_string(),
                                protocol: "modbus".to_string(),
                                method: None,
                                path: None,
                                source_file: format!(
                                    "{}:{}",
                                    file.relative_path,
                                    capture.node.start_position().row + 1
                                ),
                                confidence: Confidence::High,
                                extraction_method: "python_modbus_client".to_string(),
                                evidence: Some(stub_name.to_string()),
                            });
                        }
                    }
                }
            }
        }

        // OPC UA (opcua/asyncua)
        if (content.contains("import opcua") || content.contains("import asyncua"))
            && content.contains("Client")
        {
            let source_service = scope_to_service(&file.path, &ctx.service_roots);
            if source_service.is_none() {
                tracing::warn!("Unscoped Python file: {}", file.relative_path);
            }

            connections.push(ConnectionInfo {
                source_service: source_service.unwrap_or("").to_string(),
                target_name: "opc_server".to_string(),
                protocol: "opcua".to_string(),
                method: None,
                path: None,
                source_file: format!("{}:1", file.relative_path),
                confidence: Confidence::High,
                extraction_method: "python_opcua_client".to_string(),
                evidence: Some("OPC UA Client import detected".to_string()),
            });
        }

        // BAC0 (BACnet)
        if content.contains("import BAC0") && content.contains("BAC0.connect") {
            let source_service = scope_to_service(&file.path, &ctx.service_roots);
            if source_service.is_none() {
                tracing::warn!("Unscoped Python file: {}", file.relative_path);
            }

            connections.push(ConnectionInfo {
                source_service: source_service.unwrap_or("").to_string(),
                target_name: "bacnet_network".to_string(),
                protocol: "bacnet".to_string(),
                method: None,
                path: None,
                source_file: format!("{}:1", file.relative_path),
                confidence: Confidence::High,
                extraction_method: "python_bacnet_client".to_string(),
                evidence: Some("BAC0 BACnet import detected".to_string()),
            });
        }

        // python-can (CAN bus)
        if content.contains("import can") && content.contains("Bus") {
            let source_service = scope_to_service(&file.path, &ctx.service_roots);
            if source_service.is_none() {
                tracing::warn!("Unscoped Python file: {}", file.relative_path);
            }

            connections.push(ConnectionInfo {
                source_service: source_service.unwrap_or("").to_string(),
                target_name: "can_network".to_string(),
                protocol: "canbus".to_string(),
                method: None,
                path: None,
                source_file: format!("{}:1", file.relative_path),
                confidence: Confidence::High,
                extraction_method: "python_can_client".to_string(),
                evidence: Some("python-can Bus import detected".to_string()),
            });
        }

        // hl7apy (HL7)
        if (content.contains("import hl7apy") || content.contains("from hl7apy"))
            && content.contains("Message")
        {
            let source_service = scope_to_service(&file.path, &ctx.service_roots);
            if source_service.is_none() {
                tracing::warn!("Unscoped Python file: {}", file.relative_path);
            }

            connections.push(ConnectionInfo {
                source_service: source_service.unwrap_or("").to_string(),
                target_name: "hl7_system".to_string(),
                protocol: "hl7".to_string(),
                method: None,
                path: None,
                source_file: format!("{}:1", file.relative_path),
                confidence: Confidence::High,
                extraction_method: "python_hl7_client".to_string(),
                evidence: Some("hl7apy Message import detected".to_string()),
            });
        }
    }

    connections
}

fn extract_grpc_clients(
    ctx: &ExtractionContext,
    source_files: &[&FileContext],
    parser: &mut Parser,
) -> Vec<ConnectionInfo> {
    let mut connections = Vec::new();
    let query = grpc_stub_query();

    for file in source_files {
        let content = &*file.content;

        // Import gate: file content contains _pb2_grpc
        if !content.contains("_pb2_grpc") {
            continue;
        }

        let source_service = scope_to_service(&file.path, &ctx.service_roots);
        if source_service.is_none() {
            tracing::warn!("Unscoped Python file: {}", file.relative_path);
        }

        let source_bytes = file.content.as_bytes();
        let tree = match parser.parse(source_bytes, None) {
            Some(t) => t,
            None => continue,
        };

        let mut cursor = QueryCursor::new();
        use streaming_iterator::StreamingIterator;
        let mut matches = cursor.matches(query, tree.root_node(), source_bytes);

        while let Some(m) = matches.next() {
            let mut stub_name = "";

            for capture in m.captures {
                let name = query.capture_names()[capture.index as usize];
                if name == "stub" {
                    stub_name = capture
                        .node
                        .utf8_text(source_bytes)
                        .unwrap_or("")
                        .trim_matches('"')
                        .trim_matches('\'');
                }
            }

            // Filter: stub name ends with "Stub"
            if stub_name.ends_with("Stub") {
                let service_name = stub_name.trim_end_matches("Stub").to_string();
                connections.push(ConnectionInfo {
                    source_service: source_service.unwrap_or("").to_string(),
                    target_name: service_name,
                    protocol: "grpc".to_string(),
                    method: None,
                    path: None,
                    source_file: format!(
                        "{}:{}",
                        file.relative_path,
                        m.captures
                            .first()
                            .map(|c| c.node.start_position().row + 1)
                            .unwrap_or(0)
                    ),
                    confidence: Confidence::High,
                    extraction_method: "python_grpc_stub".to_string(),
                    evidence: Some(stub_name.to_string()),
                });
            }
        }
    }

    connections
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_file_context(relative_path: &str, content: &str) -> FileContext {
        FileContext {
            path: std::path::PathBuf::from("/repo/").join(relative_path),
            relative_path: relative_path.to_string(),
            content: Arc::from(content),
        }
    }

    fn make_extraction_context(files: Vec<FileContext>) -> ExtractionContext {
        ExtractionContext {
            files,
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        }
    }

    #[test]
    fn test_fastapi_route_detection() {
        let files = vec![
            make_file_context("requirements.txt", "fastapi==0.104.0\nuvicorn==0.24.0\n"),
            make_file_context(
                "app.py",
                "@app.get('/users')\ndef list_users():\n    pass\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert_eq!(result.endpoints.len(), 1);
        let ep = &result.endpoints[0];
        assert_eq!(ep.method, "GET");
        assert_eq!(ep.path, "/users");
        assert_eq!(ep.handler, Some("list_users".to_string()));
        assert_eq!(ep.kind, "rest");
    }

    #[test]
    fn test_django_urlpatterns_detection() {
        let files = vec![
            make_file_context("requirements.txt", "django==4.2.0\n"),
            make_file_context("urls.py", "urlpatterns = [path('users/', list_users)]\n"),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert_eq!(result.endpoints.len(), 1);
        let ep = &result.endpoints[0];
        assert_eq!(ep.method, "GET");
        assert_eq!(ep.path, "users/");
    }

    #[test]
    fn test_no_framework_marker_skips_routes() {
        let files = vec![make_file_context(
            "app.py",
            "@app.get('/users')\ndef list_users():\n    pass\n",
        )];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        // No framework marker in requirements/pyproject, so routes should not be detected
        assert_eq!(result.endpoints.len(), 0);
    }

    #[test]
    fn test_requests_http_client_detection() {
        let files = vec![
            make_file_context("requirements.txt", "requests==2.31.0\n"),
            make_file_context(
                "client.py",
                "import requests\nresponse = requests.post('http://api.example.com/data')\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "rest");
        assert_eq!(conn.target_name, "http://api.example.com/data");
    }

    #[test]
    fn test_asyncpg_database_client_detection() {
        let files = vec![
            make_file_context(
                "requirements.txt",
                "asyncpg==0.28.0\n",
            ),
            make_file_context(
                "db.py",
                "import asyncpg\nconn = await asyncpg.connect('postgresql://user:pass@localhost/db')\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert!(result
            .connections
            .iter()
            .any(|c| c.protocol == "postgresql"));
    }

    #[test]
    fn test_modbus_industrial_protocol_detection() {
        let files = vec![
            make_file_context(
                "requirements.txt",
                "pymodbus==3.0.0\n",
            ),
            make_file_context(
                "ics.py",
                "from pymodbus.client.sync import ModbusTcpClient\nclient = ModbusTcpClient(host='192.168.1.1')\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert!(result.connections.iter().any(|c| c.protocol == "modbus"));
    }

    #[test]
    fn test_grpc_stub_client_detection() {
        let files = vec![
            make_file_context("requirements.txt", "grpcio==1.50.0\n"),
            make_file_context(
                "grpc_client.py",
                "from order_pb2_grpc import OrderServiceStub\nstub = OrderServiceStub(channel)\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert!(result.connections.iter().any(|c| c.protocol == "grpc"));
    }
}
