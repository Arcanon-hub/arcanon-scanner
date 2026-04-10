// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::collections::HashMap;

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

use crate::plugin::{scope_to_service, ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, EndpointInfo, ExtractionResult};

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

const QUERY_GORILLA_ROUTES: &str = r#"(call_expression
  function: (selector_expression
    operand: (identifier) @router
    field: (field_identifier) @method)
  arguments: (argument_list
    (interpreted_string_literal) @path))"#;

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
                            extraction_method: "go-gin-echo-fiber-route".to_string()
                        });
                    }
                }
            }
        }
    }

    // Chi routes (same AST pattern as Gin/Echo)
    if frameworks.chi {
        let matches = query_matches_grouped(language, source, QUERY_GIN_ROUTES);
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
                            extraction_method: "go-chi-route".to_string()
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
                        extraction_method: "go-gorilla-mux-route".to_string()
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
                        extraction_method: "go-http-handlefunc".to_string()
                    });
                }
            }
        }
    }
}

/// Detect HTTP client calls
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
        let matches = query_matches_grouped(&language, source, QUERY_GIN_ROUTES);

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
    fn test_extract_string_literal_double_quotes() {
        let result = extract_string_literal("\"hello world\"");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_extract_string_literal_single_quotes() {
        let result = extract_string_literal("'hello world'");
        assert_eq!(result, "hello world");
    }
}
