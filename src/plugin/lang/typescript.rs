// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::collections::HashMap;
use std::sync::OnceLock;

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

use crate::plugin::{scope_to_service, ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, EndpointInfo, ExtractionResult};

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
    file_content: &str,
    relative_path: &str,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
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

        let service_name =
            scope_to_service(&std::path::PathBuf::from(relative_path), service_roots)
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

/// Extract NestJS routes using two-phase extraction
fn extract_nestjs_routes(
    result: &mut ExtractionResult,
    lang: &Language,
    file_content: &str,
    relative_path: &str,
    service_roots: &HashMap<std::path::PathBuf, String>,
) {
    let mut parser = Parser::new();
    if parser.set_language(lang).is_err() {
        return;
    }

    let tree = match parser.parse(file_content.as_bytes(), None) {
        Some(t) => t,
        None => return,
    };

    // Simplified query: detect @Get, @Post, etc. method decorators
    // This matches @Get("/:id"), @Post(), etc.
    let simple_decorator_query_str = r#"
(decorator
  (call_expression
    function: (identifier) @http_dec
    arguments: (arguments (string)? @path_arg)))
"#;

    let simple_query = match Query::new(lang, simple_decorator_query_str) {
        Ok(q) => q,
        Err(_) => return,
    };

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&simple_query, tree.root_node(), file_content.as_bytes());

    let capture_names = simple_query.capture_names();
    let http_methods = ["Get", "Post", "Put", "Delete", "Patch"];

    let service_name = scope_to_service(&std::path::PathBuf::from(relative_path), service_roots)
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
            result.endpoints.push(EndpointInfo {
                service_name: service_name.clone(),
                method: http_dec.to_uppercase(),
                path: path_arg,
                handler: None,
                kind: "rest".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast_nestjs".to_string(),
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
                    extract_express_routes(
                        &mut result,
                        &lang,
                        &file.content,
                        &file.relative_path,
                        &ctx.service_roots,
                    );
                }

                // Extract NestJS routes if framework detected
                if frameworks.nestjs {
                    extract_nestjs_routes(
                        &mut result,
                        &lang,
                        &file.content,
                        &file.relative_path,
                        &ctx.service_roots,
                    );
                }
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
        assert_eq!(endpoint.extraction_method, "ast_nestjs");
    }
}
