// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::sync::OnceLock;
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

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

/// Compile unified member call query for HTTP and database clients.
/// Captures @lib, @method, and the first string argument (captured as @arg).
/// Post-filters in detectors distinguish between HTTP and DB clients by library/method names.
fn member_call_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    QUERY.get_or_init(|| {
        let src = r#"
(call
  function: (attribute
    object: (identifier) @lib
    attribute: (identifier) @method)
  arguments: (argument_list (string) @arg (_)*))
"#;
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        Query::new(&lang, src).expect("invalid member_call query")
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

/// Compile Modbus client instantiation query once via OnceLock.
fn modbus_client_query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    QUERY.get_or_init(|| {
        let src = r#"
(call
  function: (identifier) @client
  arguments: (argument_list (_)*))
"#;
        let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
        Query::new(&lang, src).expect("invalid modbus_client query")
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
                .extend(extract_celery_connections(ctx, &source_files));
            result
                .connections
                .extend(extract_nats_connections(ctx, &source_files));
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
        let mut matches = cursor.matches(query, tree.root_node(), source_bytes);
        while let Some(m) = matches.next() {
            let mut _obj_name = "";
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
                    "obj" => _obj_name = text,
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
    let http_libs = ["requests", "httpx", "urllib"];
    let http_methods = ["get", "post", "put", "delete", "patch"];
    let query = member_call_query();

    for file in source_files {
        let source_bytes = file.content.as_bytes();
        let content = &*file.content;

        // Skip if no HTTP library import
        if !http_libs.iter().any(|lib| content.contains(lib)) {
            // Check for aiohttp separately (pattern is different)
            if !content.contains("aiohttp") {
                continue;
            }
        }

        let tree = match parser.parse(source_bytes, None) {
            Some(t) => t,
            None => continue,
        };

        let mut cursor = QueryCursor::new();
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
                    "arg" => url = text,
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

        // Handle aiohttp.ClientSession separately (line-by-line scanning)
        if content.contains("aiohttp") && content.contains("ClientSession") {
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if line.contains("session.get(")
                    || line.contains("session.post(")
                    || line.contains("session.put(")
                    || line.contains("session.delete(")
                    || line.contains("session.patch(")
                {
                    // Extract URL from quoted string after session.<method>(
                    let method_idx = line
                        .find("session.get(")
                        .map(|idx| ("get", idx))
                        .or_else(|| line.find("session.post(").map(|idx| ("post", idx)))
                        .or_else(|| line.find("session.put(").map(|idx| ("put", idx)))
                        .or_else(|| line.find("session.delete(").map(|idx| ("delete", idx)))
                        .or_else(|| line.find("session.patch(").map(|idx| ("patch", idx)));

                    if let Some((method, idx)) = method_idx {
                        let rest = &line[idx + method.len() + 9..]; // +9 for "session." + "("
                        if let Some(quote_start) = rest.find('"').or_else(|| rest.find('\'')) {
                            let quote_char = &rest[quote_start..quote_start + 1];
                            if let Some(quote_end) = rest[quote_start + 1..].find(quote_char) {
                                let url = &rest[quote_start + 1..quote_start + 1 + quote_end];
                                let source_service =
                                    scope_to_service(&file.path, &ctx.service_roots);
                                if source_service.is_none() {
                                    tracing::warn!("Unscoped Python file: {}", file.relative_path);
                                }

                                connections.push(ConnectionInfo {
                                    source_service: source_service.unwrap_or("").to_string(),
                                    target_name: url.to_string(),
                                    protocol: "rest".to_string(),
                                    method: Some(method.to_uppercase()),
                                    path: None,
                                    source_file: format!("{}:{}", file.relative_path, i + 1),
                                    confidence: Confidence::Medium,
                                    extraction_method: "python_aiohttp_client".to_string(),
                                    evidence: Some(
                                        url[..std::cmp::min(url.len(), 200)].to_string(),
                                    ),
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

fn extract_db_clients(
    ctx: &ExtractionContext,
    source_files: &[&FileContext],
    parser: &mut Parser,
) -> Vec<ConnectionInfo> {
    let mut connections = Vec::new();
    let query = member_call_query();

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
        } else if content.contains("sqlalchemy") || content.contains("create_engine") {
            // Will handle separately below
            ("", "")
        } else {
            continue;
        };

        if !lib.is_empty() {
            let tree = match parser.parse(source_bytes, None) {
                Some(t) => t,
                None => continue,
            };

            let mut cursor = QueryCursor::new();
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
                        "arg" => dsn = text,
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

        // SQLAlchemy create_engine() detection (line-by-line)
        if content.contains("sqlalchemy") && content.contains("create_engine") {
            let lines: Vec<&str> = content.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if line.contains("create_engine(") {
                    // Extract DSN from quoted string
                    if let Some(idx) = line.find("create_engine(") {
                        let rest = &line[idx + 14..]; // +14 for "create_engine("
                        if let Some(quote_start) = rest.find('"').or_else(|| rest.find('\'')) {
                            let quote_char = &rest[quote_start..quote_start + 1];
                            if let Some(quote_end) = rest[quote_start + 1..].find(quote_char) {
                                let dsn = &rest[quote_start + 1..quote_start + 1 + quote_end];
                                // Extract protocol from DSN (postgresql://, mysql://, sqlite:///)
                                let protocol = if dsn.starts_with("postgresql://") {
                                    "postgresql"
                                } else if dsn.starts_with("mysql://") {
                                    "mysql"
                                } else if dsn.starts_with("sqlite:///") {
                                    "sqlite"
                                } else if dsn.starts_with("oracle://") {
                                    "oracle"
                                } else if dsn.starts_with("mssql://") {
                                    "mssql"
                                } else {
                                    "sql"
                                };

                                let source_service =
                                    scope_to_service(&file.path, &ctx.service_roots);
                                if source_service.is_none() {
                                    tracing::warn!("Unscoped Python file: {}", file.relative_path);
                                }

                                connections.push(ConnectionInfo {
                                    source_service: source_service.unwrap_or("").to_string(),
                                    target_name: dsn.to_string(),
                                    protocol: protocol.to_string(),
                                    method: None,
                                    path: None,
                                    source_file: format!("{}:{}", file.relative_path, i + 1),
                                    confidence: Confidence::High,
                                    extraction_method: "python_sqlalchemy_client".to_string(),
                                    evidence: Some(
                                        dsn[..std::cmp::min(dsn.len(), 200)].to_string(),
                                    ),
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

fn extract_celery_connections(
    ctx: &ExtractionContext,
    source_files: &[&FileContext],
) -> Vec<ConnectionInfo> {
    let mut connections = Vec::new();

    for file in source_files {
        let content = &*file.content;

        // Celery: app = Celery(name, broker='url')
        if content.contains("celery") || content.contains("Celery") {
            if content.contains("Celery(") || content.contains("celery.Celery(") {
                let source_service = scope_to_service(&file.path, &ctx.service_roots);
                if source_service.is_none() {
                    tracing::warn!("Unscoped Python file: {}", file.relative_path);
                }

                let lines: Vec<&str> = content.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    if line.contains("Celery(") {
                        // Extract broker URL from broker='url' or broker="url"
                        if let Some(broker_idx) = line.find("broker=") {
                            let rest = &line[broker_idx + 7..]; // +7 for "broker="
                            if let Some(quote_start) = rest.find('"').or_else(|| rest.find('\'')) {
                                let quote_char = &rest[quote_start..quote_start + 1];
                                if let Some(quote_end) = rest[quote_start + 1..].find(quote_char) {
                                    let broker_url =
                                        &rest[quote_start + 1..quote_start + 1 + quote_end];
                                    // Extract protocol from broker URL
                                    let protocol = if broker_url.starts_with("redis://") {
                                        "redis"
                                    } else if broker_url.starts_with("amqp://")
                                        || broker_url.starts_with("pyamqp://")
                                    {
                                        "amqp"
                                    } else {
                                        "broker"
                                    };

                                    connections.push(ConnectionInfo {
                                        source_service: source_service.unwrap_or("").to_string(),
                                        target_name: broker_url.to_string(),
                                        protocol: protocol.to_string(),
                                        method: None,
                                        path: None,
                                        source_file: format!("{}:{}", file.relative_path, i + 1),
                                        confidence: Confidence::High,
                                        extraction_method: "python_celery_broker".to_string(),
                                        evidence: Some(
                                            broker_url[..std::cmp::min(broker_url.len(), 200)]
                                                .to_string(),
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // Also detect app.send_task('task_name') calls
            if content.contains("send_task(") {
                let source_service = scope_to_service(&file.path, &ctx.service_roots);
                let lines: Vec<&str> = content.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    if line.contains("send_task(") {
                        if let Some(idx) = line.find("send_task(") {
                            let rest = &line[idx + 10..]; // +10 for "send_task("
                            if let Some(quote_start) = rest.find('"').or_else(|| rest.find('\'')) {
                                let quote_char = &rest[quote_start..quote_start + 1];
                                if let Some(quote_end) = rest[quote_start + 1..].find(quote_char) {
                                    let task_name =
                                        &rest[quote_start + 1..quote_start + 1 + quote_end];
                                    connections.push(ConnectionInfo {
                                        source_service: source_service.unwrap_or("").to_string(),
                                        target_name: "celery_task_queue".to_string(),
                                        protocol: "celery".to_string(),
                                        method: None,
                                        path: Some(task_name.to_string()),
                                        source_file: format!("{}:{}", file.relative_path, i + 1),
                                        confidence: Confidence::High,
                                        extraction_method: "python_celery_task".to_string(),
                                        evidence: Some(task_name.to_string()),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    connections
}

fn extract_nats_connections(
    ctx: &ExtractionContext,
    source_files: &[&FileContext],
) -> Vec<ConnectionInfo> {
    let mut connections = Vec::new();

    for file in source_files {
        let content = &*file.content;

        // NATS: import nats or from nats import
        if !content.contains("nats") {
            continue;
        }

        if content.contains("import nats") || content.contains("from nats") {
            let source_service = scope_to_service(&file.path, &ctx.service_roots);
            if source_service.is_none() {
                tracing::warn!("Unscoped Python file: {}", file.relative_path);
            }

            // Detect nats.connect('url') calls
            if content.contains("nats.connect(") {
                let lines: Vec<&str> = content.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    if line.contains("nats.connect(") {
                        if let Some(idx) = line.find("nats.connect(") {
                            let rest = &line[idx + 13..]; // +13 for "nats.connect("
                            if let Some(quote_start) = rest.find('"').or_else(|| rest.find('\'')) {
                                let quote_char = &rest[quote_start..quote_start + 1];
                                if let Some(quote_end) = rest[quote_start + 1..].find(quote_char) {
                                    let url = &rest[quote_start + 1..quote_start + 1 + quote_end];
                                    connections.push(ConnectionInfo {
                                        source_service: source_service.unwrap_or("").to_string(),
                                        target_name: url.to_string(),
                                        protocol: "nats".to_string(),
                                        method: None,
                                        path: None,
                                        source_file: format!("{}:{}", file.relative_path, i + 1),
                                        confidence: Confidence::High,
                                        extraction_method: "python_nats_connect".to_string(),
                                        evidence: Some(
                                            url[..std::cmp::min(url.len(), 200)].to_string(),
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
            }

            // Detect publish/subscribe patterns
            if content.contains("publish(") || content.contains("subscribe(") {
                let lines: Vec<&str> = content.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    if line.contains(".publish(") || line.contains(".subscribe(") {
                        let (method, search_str) = if line.contains(".publish(") {
                            ("publish", ".publish(")
                        } else {
                            ("subscribe", ".subscribe(")
                        };

                        if let Some(idx) = line.find(search_str) {
                            let rest = &line[idx + search_str.len()..];
                            if let Some(quote_start) = rest.find('"').or_else(|| rest.find('\'')) {
                                let quote_char = &rest[quote_start..quote_start + 1];
                                if let Some(quote_end) = rest[quote_start + 1..].find(quote_char) {
                                    let subject =
                                        &rest[quote_start + 1..quote_start + 1 + quote_end];
                                    connections.push(ConnectionInfo {
                                        source_service: source_service.unwrap_or("").to_string(),
                                        target_name: "nats_broker".to_string(),
                                        protocol: "nats".to_string(),
                                        method: Some(method.to_uppercase()),
                                        path: Some(subject.to_string()),
                                        source_file: format!("{}:{}", file.relative_path, i + 1),
                                        confidence: Confidence::High,
                                        extraction_method: "python_nats_pubsub".to_string(),
                                        evidence: Some(subject.to_string()),
                                    });
                                }
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

            let modbus_query = modbus_client_query();
            let mut cursor = QueryCursor::new();
            let mut matches = cursor.matches(modbus_query, tree.root_node(), source_bytes);

            while let Some(m) = matches.next() {
                for capture in m.captures {
                    let name = modbus_query.capture_names()[capture.index as usize];
                    if name == "client" {
                        let client_name = capture
                            .node
                            .utf8_text(source_bytes)
                            .unwrap_or("")
                            .trim_matches('"')
                            .trim_matches('\'');

                        if client_name == "ModbusTcpClient" {
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
                                evidence: Some(client_name.to_string()),
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

    #[test]
    fn test_aiohttp_client_session_detection() {
        let files = vec![
            make_file_context("requirements.txt", "aiohttp==3.9.0\n"),
            make_file_context(
                "async_client.py",
                "import aiohttp\nasync with aiohttp.ClientSession() as session:\n    async with session.get('http://api.example.com') as resp:\n        data = await resp.json()\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert!(result
            .connections
            .iter()
            .any(|c| c.protocol == "rest" && c.target_name.contains("api.example.com")));
    }

    #[test]
    fn test_sqlalchemy_create_engine_detection() {
        let files = vec![
            make_file_context("requirements.txt", "sqlalchemy==2.0.0\n"),
            make_file_context(
                "db.py",
                "from sqlalchemy import create_engine\nengine = create_engine('postgresql://user:pass@localhost/mydb')\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert!(result.connections.iter().any(|c| c.protocol == "postgresql"
            && c.target_name.contains("postgresql://")
            && c.extraction_method == "python_sqlalchemy_client"));
    }

    #[test]
    fn test_celery_broker_detection() {
        let files = vec![
            make_file_context("requirements.txt", "celery==5.3.0\n"),
            make_file_context(
                "tasks.py",
                "from celery import Celery\napp = Celery('myapp', broker='redis://localhost:6379/0')\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert!(result.connections.iter().any(|c| c.protocol == "redis"
            && c.extraction_method == "python_celery_broker"
            && c.target_name.contains("redis://")));
    }

    #[test]
    fn test_celery_amqp_broker_detection() {
        let files = vec![
            make_file_context("requirements.txt", "celery==5.3.0\n"),
            make_file_context(
                "tasks.py",
                "from celery import Celery\napp = Celery('myapp', broker='amqp://guest:guest@localhost//')\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert!(result
            .connections
            .iter()
            .any(|c| c.protocol == "amqp" && c.extraction_method == "python_celery_broker"));
    }

    #[test]
    fn test_celery_task_send_detection() {
        let files = vec![
            make_file_context("requirements.txt", "celery==5.3.0\n"),
            make_file_context(
                "tasks.py",
                "from celery import Celery\napp = Celery('myapp')\napp.send_task('mytask.process_data')\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert!(result.connections.iter().any(|c| c.protocol == "celery"
            && c.extraction_method == "python_celery_task"
            && c.path
                .as_ref()
                .map(|p| p.contains("process_data"))
                .unwrap_or(false)));
    }

    #[test]
    fn test_nats_connect_detection() {
        let files = vec![
            make_file_context("requirements.txt", "nats-py==2.0.0\n"),
            make_file_context(
                "nats_client.py",
                "import nats\nnc = await nats.connect('nats://localhost:4222')\nawait nc.subscribe('subject.name')\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        assert!(result.connections.iter().any(|c| c.protocol == "nats"
            && c.extraction_method == "python_nats_connect"
            && c.target_name.contains("nats://localhost:4222")));
    }

    #[test]
    fn test_nats_pubsub_detection() {
        let files = vec![
            make_file_context("requirements.txt", "nats-py==2.0.0\n"),
            make_file_context(
                "nats_client.py",
                "import nats\nnc = await nats.connect('nats://localhost:4222')\nawait nc.publish('events.user.created', b'data')\nawait nc.subscribe('events.user.deleted')\n",
            ),
        ];

        let ctx = make_extraction_context(files);
        let plugin = PythonPlugin;
        let result = plugin.extract(&ctx);

        let publish_found = result.connections.iter().any(|c| {
            c.protocol == "nats"
                && c.extraction_method == "python_nats_pubsub"
                && c.method.as_ref().map(|m| m == "PUBLISH").unwrap_or(false)
                && c.path
                    .as_ref()
                    .map(|p| p.contains("events.user.created"))
                    .unwrap_or(false)
        });

        let subscribe_found = result.connections.iter().any(|c| {
            c.protocol == "nats"
                && c.extraction_method == "python_nats_pubsub"
                && c.method.as_ref().map(|m| m == "SUBSCRIBE").unwrap_or(false)
                && c.path
                    .as_ref()
                    .map(|p| p.contains("events.user.deleted"))
                    .unwrap_or(false)
        });

        assert!(publish_found && subscribe_found);
    }
}
