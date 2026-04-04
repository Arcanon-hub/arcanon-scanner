// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::collections::HashMap;

use crate::ast::QueryMatch;
use crate::plugin::{scope_to_service, ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, EndpointInfo, ExtractionResult};

/// Go language plugin.
/// Covers .go and go.mod files.
/// Detects routes (net/http, Gin, Echo, Fiber), HTTP clients, gRPC, and database connections.
pub struct GoPlugin;

/// Detected frameworks in a Go project.
#[derive(Debug, Clone)]
struct GoFrameworks {
    gin: bool,
    echo: bool,
    fiber: bool,
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

/// Detect frameworks by scanning go.mod file content.
fn detect_frameworks(files: &[crate::plugin::FileContext]) -> GoFrameworks {
    let mut frameworks = GoFrameworks {
        gin: false,
        echo: false,
        fiber: false,
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
        }
    }

    frameworks
}

/// Build an AstHelper for Go with tree_sitter_go language.
fn build_go_helper() -> crate::ast::AstHelper {
    crate::ast::AstHelper::new(tree_sitter_go::LANGUAGE.into())
}

/// Extract path from interpreted_string_literal by removing quotes.
fn extract_string_literal(text: &str) -> String {
    text.trim_matches('"').trim_matches('\'').to_string()
}

/// Helper function to group query matches by capturing groups into maps.
fn group_matches_by_query(matches: &[QueryMatch]) -> Vec<HashMap<String, String>> {
    let mut groups = Vec::new();
    let mut current_group: HashMap<String, String> = HashMap::new();

    for m in matches {
        current_group.insert(m.capture_name.clone(), m.node_text.clone());
    }

    if !current_group.is_empty() {
        groups.push(current_group);
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

        let helper = build_go_helper();

        // Process each .go file
        for file in &ctx.files {
            if !file.relative_path.ends_with(".go") {
                continue;
            }

            let source = file.content.as_ref();

            // Route detection (Gin, Echo, Fiber, net/http)
            detect_routes(&helper, file, &frameworks, ctx, &mut result, source);

            // HTTP client detection
            detect_http_clients(&helper, file, ctx, &mut result, source);

            // gRPC detection
            detect_grpc(&helper, file, ctx, &mut result, source);

            // Database connection detection
            detect_database(&helper, file, ctx, &mut result, source);
        }

        result
    }
}

/// Detect routes from Gin, Echo, Fiber, and net/http
fn detect_routes(
    helper: &crate::ast::AstHelper,
    file: &crate::plugin::FileContext,
    frameworks: &GoFrameworks,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
    source: &str,
) {
    // Gin, Echo, Fiber routes (uppercase HTTP methods)
    if frameworks.gin || frameworks.echo || frameworks.fiber {
        let matches = helper.query_matches(source, QUERY_GIN_ROUTES);
        for m in group_matches_by_query(&matches) {
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

    // net/http HandleFunc (stdlib — always available)
    let http_matches = helper.query_matches(source, QUERY_HTTP_HANDLEFUNC);
    for m in group_matches_by_query(&http_matches) {
        if let (Some(pkg), Some(fn_name), Some(path)) = (m.get("pkg"), m.get("fn"), m.get("path")) {
            let valid_pkg = ["http", "mux", "r", "router"].contains(&pkg);
            let valid_fn = ["HandleFunc", "Handle"].contains(&fn_name);

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
    helper: &crate::ast::AstHelper,
    file: &crate::plugin::FileContext,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
    source: &str,
) {
    let matches = helper.query_matches(source, QUERY_HTTP_CLIENT);

    for m in group_matches_by_query(&matches) {
        if let (Some(obj), Some(method)) = (m.get("obj"), m.get("method")) {
            let valid_obj = ["http", "client", "c", "httpClient"].contains(&obj);
            let valid_method =
                ["Get", "Post", "Do", "NewRequest", "Head", "Put", "Delete"].contains(&method);

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
    helper: &crate::ast::AstHelper,
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

    let matches = helper.query_matches(source, QUERY_GRPC_DIAL);

    for m in group_matches_by_query(&matches) {
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

/// Detect database connections
fn detect_database(
    helper: &crate::ast::AstHelper,
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

    let matches = helper.query_matches(source, QUERY_SQL_OPEN);

    for m in group_matches_by_query(&matches) {
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
    fn test_gin_route_detection() {
        let helper = build_go_helper();
        let source = r#"
func main() {
    r := gin.Default()
    r.GET("/users", getUsers)
    r.POST("/users", createUser)
}
"#;
        let matches = helper.query_matches(source, QUERY_GIN_ROUTES);

        let path_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.capture_name == "path")
            .collect();

        assert!(!path_matches.is_empty(), "Should detect Gin routes");
    }

    #[test]
    fn test_http_handlefunc_detection() {
        let helper = build_go_helper();
        let source = r#"
func main() {
    http.HandleFunc("/health", healthCheck)
    http.HandleFunc("/api/users", getUsers)
}
"#;
        let matches = helper.query_matches(source, QUERY_HTTP_HANDLEFUNC);

        let path_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.capture_name == "path")
            .collect();

        assert!(
            !path_matches.is_empty(),
            "Should detect http.HandleFunc calls"
        );
    }

    #[test]
    fn test_http_client_detection() {
        let helper = build_go_helper();
        let source = r#"
func main() {
    resp, _ := http.Get("http://example.com/api")
    http.Post("http://api.example.com/data", "application/json", body)
}
"#;
        let matches = helper.query_matches(source, QUERY_HTTP_CLIENT);

        let url_matches: Vec<_> = matches.iter().filter(|m| m.capture_name == "url").collect();

        assert!(!url_matches.is_empty(), "Should detect http.Get/Post calls");
    }

    #[test]
    fn test_grpc_dial_detection() {
        let helper = build_go_helper();
        let source = r#"
import "google.golang.org/grpc"

func main() {
    conn, _ := grpc.Dial("auth-svc:50051")
    conn2, _ := grpc.DialContext(ctx, "user-svc:50052")
}
"#;
        let matches = helper.query_matches(source, QUERY_GRPC_DIAL);

        let addr_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.capture_name == "addr")
            .collect();

        assert!(!addr_matches.is_empty(), "Should detect grpc.Dial calls");
    }

    #[test]
    fn test_kafka_producer_detection() {
        let helper = build_go_helper();
        let source = r#"
import "github.com/segmentio/kafka-go"

func main() {
    w := kafka.NewWriter(kafka.WriterConfig{})
    w.WriteMessages(ctx, message1, message2)
    producer.Produce(msg)
}
"#;
        let matches = helper.query_matches(source, QUERY_KAFKA_PRODUCER);

        let method_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.capture_name == "method")
            .collect();

        assert!(
            !method_matches.is_empty(),
            "Should detect kafka producer calls"
        );
    }

    #[test]
    fn test_sql_open_detection() {
        let helper = build_go_helper();
        let source = r#"
import "database/sql"

func main() {
    db, _ := sql.Open("postgres", "postgres://user:pass@localhost/db")
    db2, _ := sql.Open("mysql", "user:pass@tcp(localhost:3306)/db")
}
"#;
        let matches = helper.query_matches(source, QUERY_SQL_OPEN);

        let driver_matches: Vec<_> = matches
            .iter()
            .filter(|m| m.capture_name == "driver")
            .collect();

        assert!(!driver_matches.is_empty(), "Should detect sql.Open calls");
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
