// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::collections::HashMap;

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

use crate::plugin::{scope_to_service, ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, EndpointInfo, ExtractionResult};

/// Go language plugin.
/// Covers .go and go.mod files.
/// Detects routes (net/http, Gin, Echo, Fiber, Chi, Gorilla), HTTP clients, gRPC, NATS, MongoDB, Redis, and database connections.
pub struct GoPlugin;

/// Detected frameworks in a Go project.
#[derive(Debug, Clone)]
struct GoFrameworks {
    gin: bool,
    echo: bool,
    fiber: bool,
    chi: bool,
    gorilla_mux: bool,
    nats: bool,
    mongo: bool,
    redis: bool,
}

/// Query constants for tree-sitter Go grammar.
const QUERY_GIN_ROUTES: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @router
    field: (field_identifier) @method)
  arguments: (argument_list
    (interpreted_string_literal) @path
    (_)+ @handlers))"#;

const QUERY_HTTP_HANDLEFUNC: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @fn)
  arguments: (argument_list
    (interpreted_string_literal) @path
    (_) @handler))"#;

const QUERY_HTTP_CLIENT: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @obj
    field: (field_identifier) @method)
  arguments: (argument_list
    (interpreted_string_literal) @url))"#;

const QUERY_GRPC_DIAL: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @fn)
  arguments: (argument_list
    (interpreted_string_literal) @addr
    (_)*))"#;

const QUERY_SQL_OPEN: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @fn)
  arguments: (argument_list
    (interpreted_string_literal) @driver
    (_)))"#;

const QUERY_KAFKA_PRODUCER: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @obj
    field: (field_identifier) @method))"#;

const QUERY_CHI_ROUTES: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @router
    field: (field_identifier) @method)
  arguments: (argument_list
    (interpreted_string_literal) @path
    (_)+ @handlers))"#;

const QUERY_GORILLA_ROUTES: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @router
    field: (field_identifier) @method)
  arguments: (argument_list
    (interpreted_string_literal) @path))"#;

const QUERY_NATS_CONNECT: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @fn)
  arguments: (argument_list
    (interpreted_string_literal) @url))"#;

const QUERY_NATS_SUBSCRIBE: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @obj
    field: (field_identifier) @method)
  arguments: (argument_list
    (interpreted_string_literal) @subject
    (_)*))"#;

const QUERY_MONGO_CONNECT: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @fn)
  arguments: (_)*)"#;

const QUERY_REDIS_CLIENT: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @pkg
    field: (field_identifier) @fn)
  arguments: (argument_list (_)*))"#;

/// Detect frameworks by scanning go.mod file content.
fn detect_frameworks(files: &[crate::plugin::FileContext]) -> GoFrameworks {
    let mut frameworks = GoFrameworks {
        gin: false,
        echo: false,
        fiber: false,
        chi: false,
        gorilla_mux: false,
        nats: false,
        mongo: false,
        redis: false,
    };

    for file in files {
        if file.relative_path.ends_with("go.mod") {
            if file.content.contains("github.com/gin-gonic/gin") {
                frameworks.gin = true;
            }
            if file.content.contains("github.com/labstack/echo") {
                frameworks.echo = true;
            }
            if file.content.contains("github.com/gofiber/fiber") {
                frameworks.fiber = true;
            }
            if file.content.contains("go-chi/chi") {
                frameworks.chi = true;
            }
            if file.content.contains("gorilla/mux") {
                frameworks.gorilla_mux = true;
            }
            if file.content.contains("nats.go") {
                frameworks.nats = true;
            }
            if file.content.contains("go.mongodb.org/mongo-driver") {
                frameworks.mongo = true;
            }
            if file.content.contains("go-redis") {
                frameworks.redis = true;
            }
        }
    }

    frameworks
}

/// Build a Language for Go with tree_sitter_go.
fn build_go_language() -> Language {
    tree_sitter_go::LANGUAGE.into()
}

/// Extract path from interpreted_string_literal by removing quotes.
fn extract_string_literal(text: &str) -> String {
    text.trim_matches('"').trim_matches('\'').to_string()
}

/// Execute a query and group captures by match boundary.
/// Returns Vec<HashMap> where each HashMap is one match with all its captures.
/// This properly handles multiple matches in a single file (the critical bug fix).
fn query_matches_grouped(
    language: &Language,
    source: &str,
    query_str: &str,
) -> Vec<HashMap<String, String>> {
    let query = match Query::new(language, query_str) {
        Ok(q) => q,
        Err(e) => {
            tracing::error!("Invalid tree-sitter query: {e}");
            return vec![];
        }
    };

    let mut parser = Parser::new();
    if parser.set_language(language).is_err() {
        tracing::error!("Failed to set parser language");
        return vec![];
    }

    let tree = match parser.parse(source.as_bytes(), None) {
        Some(t) => t,
        None => {
            tracing::debug!("Failed to parse source (grammar error or too large)");
            return vec![];
        }
    };

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    let capture_names = query.capture_names();
    let mut groups = Vec::new();

    // Iterate through each match (proper match boundaries!)
    while let Some(m) = matches.next() {
        let mut group: HashMap<String, String> = HashMap::new();

        for capture in m.captures {
            let name = capture_names[capture.index as usize].to_string();
            let text = capture
                .node
                .utf8_text(source.as_bytes())
                .unwrap_or("")
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            group.insert(name, text);
        }

        if !group.is_empty() {
            groups.push(group);
        }
    }

    groups
}

impl LanguagePlugin for GoPlugin {
    fn name(&self) -> &str {
        "go"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.go", "**/go.mod"]
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        // Detect frameworks from go.mod
        let frameworks = detect_frameworks(&ctx.files);

        let language = build_go_language();

        // Process each .go file
        for file in &ctx.files {
            if !file.relative_path.ends_with(".go") {
                continue;
            }

            let source = file.content.as_ref();

            // Route detection (Gin, Echo, Fiber, Chi, Gorilla, net/http)
            detect_routes(&language, file, &frameworks, ctx, &mut result, source);

            // HTTP client detection
            detect_http_clients(&language, file, ctx, &mut result, source);

            // gRPC detection
            detect_grpc(&language, file, ctx, &mut result, source);

            // Kafka detection
            detect_kafka(&language, file, ctx, &mut result, source);

            // NATS detection
            detect_nats(&language, file, &frameworks, ctx, &mut result, source);

            // MongoDB detection
            detect_mongodb(&language, file, &frameworks, ctx, &mut result, source);

            // Redis detection
            detect_redis(&language, file, &frameworks, ctx, &mut result, source);

            // Database connection detection
            detect_database(&language, file, ctx, &mut result, source);
        }

        result
    }
}

/// Detect routes from Gin, Echo, Fiber, Chi, Gorilla, and net/http
fn detect_routes(
    language: &Language,
    file: &crate::plugin::FileContext,
    frameworks: &GoFrameworks,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
    source: &str,
) {
    // Gin, Echo, Fiber routes (uppercase HTTP methods)
    if frameworks.gin || frameworks.echo || frameworks.fiber {
        let matches = query_matches_grouped(language, source, QUERY_GIN_ROUTES);
        for m in matches {
            if let (Some(method), Some(path)) = (m.get("method"), m.get("path")) {
                let method_upper = method.to_uppercase();
                let allowed_methods = [
                    "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "ANY", "HANDLE",
                    "USE",
                ];

                if allowed_methods.contains(&method_upper.as_str())
                    || allowed_methods.iter().any(|&m| method_upper.contains(m))
                {
                    let path_str = extract_string_literal(path);
                    if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                        result.endpoints.push(EndpointInfo {
                            service_name: service_name.to_string(),
                            method: method_upper.to_string(),
                            path: path_str,
                            handler: None,
                            kind: "rest".to_string(),
                            confidence: Confidence::High,
                            extraction_method: "go-gin-echo-fiber-route".to_string(),
                        });
                    }
                }
            }
        }
    }

    // Chi routes (same AST pattern as Gin/Echo)
    if frameworks.chi {
        let matches = query_matches_grouped(language, source, QUERY_CHI_ROUTES);
        for m in matches {
            if let (Some(method), Some(path)) = (m.get("method"), m.get("path")) {
                let method_upper = method.to_uppercase();
                let allowed_methods = [
                    "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "HANDLE",
                ];

                if allowed_methods.contains(&method_upper.as_str())
                    || allowed_methods.iter().any(|&m| method_upper.contains(m))
                {
                    let path_str = extract_string_literal(path);
                    if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                        result.endpoints.push(EndpointInfo {
                            service_name: service_name.to_string(),
                            method: method_upper.to_string(),
                            path: path_str,
                            handler: None,
                            kind: "rest".to_string(),
                            confidence: Confidence::High,
                            extraction_method: "go-chi-route".to_string(),
                        });
                    }
                }
            }
        }
    }

    // Gorilla/mux routes
    if frameworks.gorilla_mux {
        let matches = query_matches_grouped(language, source, QUERY_GORILLA_ROUTES);
        for m in matches {
            if let Some(path) = m.get("path") {
                let path_str = extract_string_literal(path);
                if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                    result.endpoints.push(EndpointInfo {
                        service_name: service_name.to_string(),
                        method: "".to_string(), // .Methods() call needs separate parsing
                        path: path_str,
                        handler: None,
                        kind: "rest".to_string(),
                        confidence: Confidence::Medium,
                        extraction_method: "go-gorilla-mux-route".to_string(),
                    });
                }
            }
        }
    }

    // net/http HandleFunc (stdlib — always available)
    let http_matches = query_matches_grouped(language, source, QUERY_HTTP_HANDLEFUNC);
    for m in http_matches {
        if let (Some(pkg), Some(fn_name), Some(path)) = (m.get("pkg"), m.get("fn"), m.get("path")) {
            let valid_pkg = ["http", "mux", "r", "router"].contains(&pkg.as_str());
            let valid_fn = ["HandleFunc", "Handle"].contains(&fn_name.as_str());

            if valid_pkg && valid_fn {
                let path_str = extract_string_literal(path);
                if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                    result.endpoints.push(EndpointInfo {
                        service_name: service_name.to_string(),
                        method: "".to_string(), // HandleFunc doesn't specify method
                        path: path_str,
                        handler: None,
                        kind: "rest".to_string(),
                        confidence: Confidence::Medium,
                        extraction_method: "go-http-handlefunc".to_string(),
                    });
                }
            }
        }
    }
}

/// Detect HTTP client calls
fn detect_http_clients(
    language: &Language,
    file: &crate::plugin::FileContext,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
    source: &str,
) {
    let matches = query_matches_grouped(language, source, QUERY_HTTP_CLIENT);

    for m in matches {
        if let (Some(obj), Some(method)) = (m.get("obj"), m.get("method")) {
            let valid_obj = ["http", "client", "c", "httpClient"].contains(&obj.as_str());
            let valid_method = ["Get", "Post", "Do", "NewRequest", "Head", "Put", "Delete"]
                .contains(&method.as_str());

            if valid_obj && valid_method {
                if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                    result.connections.push(ConnectionInfo {
                        source_service: service_name.to_string(),
                        target_name: "external-http-service".to_string(),
                        protocol: "rest".to_string(),
                        method: Some(method.to_string()),
                        path: None,
                        source_file: format!("{}:1", file.relative_path),
                        confidence: Confidence::Medium,
                        extraction_method: "go-http-client".to_string(),
                        evidence: None,
                    });
                }
            }
        }
    }
}

/// Detect gRPC Dial calls
fn detect_grpc(
    language: &Language,
    file: &crate::plugin::FileContext,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
    source: &str,
) {
    // Gate: check for grpc import
    if !source.contains("google.golang.org/grpc") && !source.contains("\"google.golang.org/grpc\"")
    {
        return;
    }

    let matches = query_matches_grouped(language, source, QUERY_GRPC_DIAL);

    for m in matches {
        if let (Some(pkg), Some(fn_name), Some(addr)) = (m.get("pkg"), m.get("fn"), m.get("addr")) {
            if pkg == "grpc"
                && (fn_name == "Dial" || fn_name == "DialContext" || fn_name == "NewClient")
            {
                let addr_str = extract_string_literal(addr);
                let target_name = addr_str.split(':').next().unwrap_or(&addr_str).to_string();

                if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                    result.connections.push(ConnectionInfo {
                        source_service: service_name.to_string(),
                        target_name,
                        protocol: "grpc".to_string(),
                        method: None,
                        path: None,
                        source_file: format!("{}:1", file.relative_path),
                        confidence: Confidence::High,
                        extraction_method: "go-grpc-dial".to_string(),
                        evidence: Some(format!("grpc.{}(\"{}\")", fn_name, addr_str)),
                    });
                }
            }
        }
    }
}

/// Detect Kafka producer calls
fn detect_kafka(
    language: &Language,
    file: &crate::plugin::FileContext,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
    source: &str,
) {
    // Gate: check for Kafka library in dependencies
    // For now, we check source for kafka imports
    if !source.contains("kafka") && !source.contains("Kafka") {
        return;
    }

    let matches = query_matches_grouped(language, source, QUERY_KAFKA_PRODUCER);

    for m in matches {
        if let (Some(obj), Some(method)) = (m.get("obj"), m.get("method")) {
            let valid_method = [
                "WriteMessages",
                "Write",
                "Produce",
                "ProduceMessage",
                "Send",
            ]
            .contains(&method.as_str());

            if valid_method {
                if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                    result.connections.push(ConnectionInfo {
                        source_service: service_name.to_string(),
                        target_name: "kafka-broker".to_string(),
                        protocol: "kafka".to_string(),
                        method: Some(method.to_string()),
                        path: None,
                        source_file: format!("{}:1", file.relative_path),
                        confidence: Confidence::Medium,
                        extraction_method: "go-kafka-producer".to_string(),
                        evidence: Some(format!("{}.{}(...)", obj, method)),
                    });
                }
            }
        }
    }
}

/// Detect NATS connections
fn detect_nats(
    language: &Language,
    file: &crate::plugin::FileContext,
    frameworks: &GoFrameworks,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
    source: &str,
) {
    if !frameworks.nats {
        return;
    }

    // Gate: check for nats import
    if !source.contains("nats") && !source.contains("\"nats\"") {
        return;
    }

    // Detect Connect calls
    let matches = query_matches_grouped(language, source, QUERY_NATS_CONNECT);
    for m in matches {
        if let (Some(pkg), Some(fn_name), Some(url)) = (m.get("pkg"), m.get("fn"), m.get("url")) {
            if pkg == "nats" && fn_name == "Connect" {
                let url_str = extract_string_literal(url);
                let target_name = url_str
                    .split("://")
                    .nth(1)
                    .and_then(|s| s.split(':').next())
                    .unwrap_or(&url_str)
                    .to_string();

                if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                    result.connections.push(ConnectionInfo {
                        source_service: service_name.to_string(),
                        target_name,
                        protocol: "nats".to_string(),
                        method: None,
                        path: None,
                        source_file: format!("{}:1", file.relative_path),
                        confidence: Confidence::High,
                        extraction_method: "go-nats-connect".to_string(),
                        evidence: Some(format!("nats.Connect(\"{}\")", url_str)),
                    });
                }
            }
        }
    }

    // Detect Subscribe calls
    let matches = query_matches_grouped(language, source, QUERY_NATS_SUBSCRIBE);
    for m in matches {
        if let (Some(obj), Some(method), Some(subject)) =
            (m.get("obj"), m.get("method"), m.get("subject"))
        {
            if method == "Subscribe" || method == "QueueSubscribe" {
                let subject_str = extract_string_literal(subject);

                if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                    result.connections.push(ConnectionInfo {
                        source_service: service_name.to_string(),
                        target_name: "nats-broker".to_string(),
                        protocol: "nats".to_string(),
                        method: Some(method.to_string()),
                        path: Some(subject_str),
                        source_file: format!("{}:1", file.relative_path),
                        confidence: Confidence::High,
                        extraction_method: "go-nats-subscribe".to_string(),
                        evidence: Some(format!("{}.{}(\"{}\")", obj, method, subject)),
                    });
                }
            }
        }
    }
}

/// Detect MongoDB connections
fn detect_mongodb(
    language: &Language,
    file: &crate::plugin::FileContext,
    frameworks: &GoFrameworks,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
    source: &str,
) {
    if !frameworks.mongo {
        return;
    }

    // Gate: check for mongo-driver import
    if !source.contains("mongo") || !source.contains("Connect") {
        return;
    }

    let matches = query_matches_grouped(language, source, QUERY_MONGO_CONNECT);
    for m in matches {
        if let (Some(pkg), Some(fn_name)) = (m.get("pkg"), m.get("fn")) {
            if pkg == "mongo" && fn_name == "Connect" {
                if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                    result.connections.push(ConnectionInfo {
                        source_service: service_name.to_string(),
                        target_name: "mongodb-server".to_string(),
                        protocol: "mongodb".to_string(),
                        method: None,
                        path: None,
                        source_file: format!("{}:1", file.relative_path),
                        confidence: Confidence::High,
                        extraction_method: "go-mongo-connect".to_string(),
                        evidence: Some("mongo.Connect(ctx, options)".to_string()),
                    });
                }
            }
        }
    }
}

/// Detect Redis connections
fn detect_redis(
    language: &Language,
    file: &crate::plugin::FileContext,
    frameworks: &GoFrameworks,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
    source: &str,
) {
    if !frameworks.redis {
        return;
    }

    // Gate: check for redis import
    if !source.contains("redis") {
        return;
    }

    let matches = query_matches_grouped(language, source, QUERY_REDIS_CLIENT);
    for m in matches {
        if let (Some(pkg), Some(fn_name)) = (m.get("pkg"), m.get("fn")) {
            if pkg == "redis" && fn_name == "NewClient" {
                if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                    result.connections.push(ConnectionInfo {
                        source_service: service_name.to_string(),
                        target_name: "redis-server".to_string(),
                        protocol: "redis".to_string(),
                        method: None,
                        path: None,
                        source_file: format!("{}:1", file.relative_path),
                        confidence: Confidence::High,
                        extraction_method: "go-redis-client".to_string(),
                        evidence: Some("redis.NewClient(&redis.Options{...})".to_string()),
                    });
                }
            }
        }
    }
}

/// Detect database connections
fn detect_database(
    language: &Language,
    file: &crate::plugin::FileContext,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
    source: &str,
) {
    // Gate: check for database imports
    let has_sql = source.contains("database/sql") || source.contains("\"database/sql\"");
    let has_sqlx = source.contains("sqlx") || source.contains("jmoiern/sqlx");

    if !has_sql && !has_sqlx {
        return;
    }

    let matches = query_matches_grouped(language, source, QUERY_SQL_OPEN);

    for m in matches {
        if let (Some(pkg), Some(fn_name), Some(driver)) =
            (m.get("pkg"), m.get("fn"), m.get("driver"))
        {
            let valid =
                (pkg == "sql" || pkg == "sqlx") && (fn_name == "Open" || fn_name == "Connect");

            if valid {
                let driver_str = extract_string_literal(driver);
                let protocol = map_driver_to_protocol(&driver_str);

                if let Some(service_name) = scope_to_service(&file.path, &ctx.service_roots) {
                    result.connections.push(ConnectionInfo {
                        source_service: service_name.to_string(),
                        target_name: format!("{}-db", protocol),
                        protocol: protocol.clone(),
                        method: None,
                        path: None,
                        source_file: format!("{}:1", file.relative_path),
                        confidence: Confidence::High,
                        extraction_method: "go-sql-open".to_string(),
                        evidence: Some(format!("{}{}(\"{}\")", pkg, fn_name, driver_str)),
                    });
                }
            }
        }
    }
}

/// Map database driver string to protocol name.
fn map_driver_to_protocol(driver: &str) -> String {
    match driver {
        "postgres" | "pgx" | "postgresql" => "postgresql".to_string(),
        "mysql" | "mariadb" => "mysql".to_string(),
        "sqlite3" | "sqlite" => "sqlite".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn make_file_context(relative_path: &str, content: &str) -> crate::plugin::FileContext {
        crate::plugin::FileContext {
            path: PathBuf::from(format!("/repo/{}", relative_path)),
            relative_path: relative_path.to_string(),
            content: Arc::from(content),
        }
    }

    #[test]
    fn test_detect_frameworks_gin() {
        let files = vec![make_file_context(
            "go.mod",
            "module example.com/api\nrequire github.com/gin-gonic/gin v1.9.0",
        )];
        let fw = detect_frameworks(&files);
        assert!(fw.gin);
        assert!(!fw.echo);
        assert!(!fw.fiber);
    }

    #[test]
    fn test_detect_frameworks_echo() {
        let files = vec![make_file_context(
            "go.mod",
            "module example.com\nrequire github.com/labstack/echo v4.0.0",
        )];
        let fw = detect_frameworks(&files);
        assert!(!fw.gin);
        assert!(fw.echo);
        assert!(!fw.fiber);
    }

    #[test]
    fn test_detect_frameworks_fiber() {
        let files = vec![make_file_context(
            "go.mod",
            "module example.com\nrequire github.com/gofiber/fiber v2.0.0",
        )];
        let fw = detect_frameworks(&files);
        assert!(!fw.gin);
        assert!(!fw.echo);
        assert!(fw.fiber);
    }

    #[test]
    fn test_detect_frameworks_none() {
        let files = vec![make_file_context(
            "go.mod",
            "module example.com\nrequire golang.org/x/net v0.0.0",
        )];
        let fw = detect_frameworks(&files);
        assert!(!fw.gin);
        assert!(!fw.echo);
        assert!(!fw.fiber);
    }

    #[test]
    fn test_detect_frameworks_chi() {
        let files = vec![make_file_context(
            "go.mod",
            "module example.com\nrequire go-chi/chi v5.0.0",
        )];
        let fw = detect_frameworks(&files);
        assert!(fw.chi);
    }

    #[test]
    fn test_detect_frameworks_gorilla_mux() {
        let files = vec![make_file_context(
            "go.mod",
            "module example.com\nrequire gorilla/mux v1.8.0",
        )];
        let fw = detect_frameworks(&files);
        assert!(fw.gorilla_mux);
    }

    #[test]
    fn test_detect_frameworks_nats() {
        let files = vec![make_file_context(
            "go.mod",
            "module example.com\nrequire nats.go v1.12.0",
        )];
        let fw = detect_frameworks(&files);
        assert!(fw.nats);
    }

    #[test]
    fn test_detect_frameworks_mongo() {
        let files = vec![make_file_context(
            "go.mod",
            "module example.com\nrequire go.mongodb.org/mongo-driver v1.0.0",
        )];
        let fw = detect_frameworks(&files);
        assert!(fw.mongo);
    }

    #[test]
    fn test_detect_frameworks_redis() {
        let files = vec![make_file_context(
            "go.mod",
            "module example.com\nrequire go-redis v8.0.0",
        )];
        let fw = detect_frameworks(&files);
        assert!(fw.redis);
    }

    #[test]
    fn test_multiple_routes_in_one_file() {
        // Critical bug fix: verify that multiple routes in one file are all detected
        let language = build_go_language();
        let source = r#"
func main() {
    r := gin.Default()
    r.GET("/users", getUsers)
    r.POST("/users", createUser)
    r.PUT("/users/:id", updateUser)
    r.DELETE("/users/:id", deleteUser)
    r.GET("/health", health)
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_GIN_ROUTES);

        // Should have 5 matches (one per route), not 1
        assert_eq!(
            matches.len(),
            5,
            "Should detect all 5 routes (bug fix: not merge into single group)"
        );

        // Verify we can extract different paths from each
        let paths: Vec<_> = matches
            .iter()
            .filter_map(|m| m.get("path").map(|p| extract_string_literal(p)))
            .collect();
        assert!(paths.contains(&"/users".to_string()));
        assert!(paths.contains(&"/users/:id".to_string()));
        assert!(paths.contains(&"/health".to_string()));
    }

    #[test]
    fn test_gin_route_detection() {
        let language = build_go_language();
        let source = r#"
func main() {
    r := gin.Default()
    r.GET("/users", getUsers)
    r.POST("/users", createUser)
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_GIN_ROUTES);

        assert!(!matches.is_empty(), "Should detect Gin routes");
    }

    #[test]
    fn test_chi_route_detection() {
        let language = build_go_language();
        let source = r#"
func main() {
    r := chi.NewRouter()
    r.Get("/api/users", getUsers)
    r.Post("/api/users", createUser)
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_CHI_ROUTES);

        assert!(!matches.is_empty(), "Should detect chi router method calls");

        let paths: Vec<_> = matches
            .iter()
            .filter_map(|m| m.get("path").map(|p| extract_string_literal(p)))
            .collect();
        assert!(paths.contains(&"/api/users".to_string()));
    }

    #[test]
    fn test_gorilla_mux_detection() {
        let language = build_go_language();
        let source = r#"
func main() {
    r := mux.NewRouter()
    r.HandleFunc("/users", getUsers).Methods("GET")
    r.HandleFunc("/users", createUser).Methods("POST")
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_GORILLA_ROUTES);

        assert!(
            !matches.is_empty(),
            "Should detect gorilla/mux HandleFunc calls"
        );

        let paths: Vec<_> = matches
            .iter()
            .filter_map(|m| m.get("path").map(|p| extract_string_literal(p)))
            .collect();
        assert!(paths.contains(&"/users".to_string()));
    }

    #[test]
    fn test_http_handlefunc_detection() {
        let language = build_go_language();
        let source = r#"
func main() {
    http.HandleFunc("/health", healthCheck)
    http.HandleFunc("/api/users", getUsers)
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_HTTP_HANDLEFUNC);

        assert!(!matches.is_empty(), "Should detect http.HandleFunc calls");
    }

    #[test]
    fn test_http_client_detection() {
        let language = build_go_language();
        let source = r#"
func main() {
    resp, _ := http.Get("http://example.com/api")
    http.Post("http://api.example.com/data", "application/json", body)
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_HTTP_CLIENT);

        assert!(!matches.is_empty(), "Should detect http.Get/Post calls");
    }

    #[test]
    fn test_grpc_dial_detection() {
        let language = build_go_language();
        let source = r#"
import "google.golang.org/grpc"

func main() {
    conn, _ := grpc.Dial("auth-svc:50051")
    conn2, _ := grpc.DialContext(ctx, "user-svc:50052")
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_GRPC_DIAL);

        assert!(!matches.is_empty(), "Should detect grpc.Dial calls");
    }

    #[test]
    fn test_kafka_producer_detection() {
        let language = build_go_language();
        let source = r#"
import "github.com/segmentio/kafka-go"

func main() {
    w := kafka.NewWriter(kafka.WriterConfig{})
    w.WriteMessages(ctx, message1, message2)
    producer.Produce(msg)
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_KAFKA_PRODUCER);

        assert!(!matches.is_empty(), "Should detect kafka producer calls");
    }

    #[test]
    fn test_nats_connect_detection() {
        let language = build_go_language();
        let source = r#"
import "github.com/nats-io/nats.go"

func main() {
    nc, _ := nats.Connect("nats://localhost:4222")
    nc.Publish("subject.name", []byte("msg"))
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_NATS_CONNECT);

        assert!(!matches.is_empty(), "Should detect nats.Connect calls");

        let urls: Vec<_> = matches
            .iter()
            .filter_map(|m| m.get("url").map(|u| extract_string_literal(u)))
            .collect();
        assert!(urls.contains(&"nats://localhost:4222".to_string()));
    }

    #[test]
    fn test_nats_subscribe_detection() {
        let language = build_go_language();
        let source = r#"
import "github.com/nats-io/nats.go"

func main() {
    nc, _ := nats.Connect("nats://localhost:4222")
    sub, _ := nc.Subscribe("orders.created", messageHandler)
    qsub, _ := nc.QueueSubscribe("orders.deleted", "workers", handler)
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_NATS_SUBSCRIBE);

        assert!(
            !matches.is_empty(),
            "Should detect nats Subscribe/QueueSubscribe calls"
        );
    }

    #[test]
    fn test_mongodb_connect_detection() {
        let language = build_go_language();
        let source = r#"
import "go.mongodb.org/mongo-driver/mongo"

func main() {
    client, _ := mongo.Connect(ctx, options.Client().ApplyURI("mongodb://localhost:27017"))
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_MONGO_CONNECT);

        assert!(!matches.is_empty(), "Should detect mongo.Connect calls");
    }

    #[test]
    fn test_redis_client_detection() {
        let language = build_go_language();
        let source = r#"
import "github.com/go-redis/redis/v8"

func main() {
    rdb := redis.NewClient(&redis.Options{
        Addr: "localhost:6379",
    })
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_REDIS_CLIENT);

        assert!(!matches.is_empty(), "Should detect redis.NewClient calls");
    }

    #[test]
    fn test_sql_open_detection() {
        let language = build_go_language();
        let source = r#"
import "database/sql"

func main() {
    db, _ := sql.Open("postgres", "postgres://user:pass@localhost/db")
    db2, _ := sql.Open("mysql", "user:pass@tcp(localhost:3306)/db")
}
"#;
        let matches = query_matches_grouped(&language, source, QUERY_SQL_OPEN);

        assert!(!matches.is_empty(), "Should detect sql.Open calls");
    }

    #[test]
    fn test_extract_string_literal_double_quotes() {
        let result = extract_string_literal("\"hello world\"");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_extract_string_literal_single_quotes() {
        let result = extract_string_literal("'hello world'");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_map_driver_to_protocol_postgres() {
        assert_eq!(map_driver_to_protocol("postgres"), "postgresql");
        assert_eq!(map_driver_to_protocol("pgx"), "postgresql");
        assert_eq!(map_driver_to_protocol("postgresql"), "postgresql");
    }

    #[test]
    fn test_map_driver_to_protocol_mysql() {
        assert_eq!(map_driver_to_protocol("mysql"), "mysql");
        assert_eq!(map_driver_to_protocol("mariadb"), "mysql");
    }

    #[test]
    fn test_map_driver_to_protocol_sqlite() {
        assert_eq!(map_driver_to_protocol("sqlite3"), "sqlite");
        assert_eq!(map_driver_to_protocol("sqlite"), "sqlite");
    }

    #[test]
    fn test_map_driver_to_protocol_unknown() {
        assert_eq!(map_driver_to_protocol("custom_driver"), "custom_driver");
    }
}
