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

// OnceLock query cache for Express routes (KEEP - used for route extraction)
static EXPRESS_QUERY: OnceLock<Query> = OnceLock::new();

fn express_query(lang: &Language) -> &'static Query {
    EXPRESS_QUERY
        .get_or_init(|| Query::new(lang, ROUTE_QUERY_EXPRESS).expect("valid express query"))
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
            extraction_method: "ast_express".to_string()
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
                    extraction_method: "ast_nestjs_two_phase".to_string()
                });
            }
        }
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
            extraction_method: "ast_fastify".to_string()
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
            extraction_method: "nextjs_api_routes".to_string()
        });
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

                // Extract Next.js API routes
                extract_nextjs_routes(&mut result, file, &ctx.service_roots);
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
        assert_eq!(
            endpoint.path, "/users/:id",
            "NestJS full path must combine controller prefix + method path"
        );
    }

    #[test]
    fn test_nestjs_route_no_prefix() {
        let plugin = TypeScriptPlugin;

        let package_json = FileContext {
            path: std::path::PathBuf::from("/repo/package.json"),
            relative_path: "package.json".to_string(),
            content: Arc::from(r#"{"dependencies": {"@nestjs/core": "^10.0"}}"#),
        };

        let controller_file = FileContext {
            path: std::path::PathBuf::from("/repo/health.controller.ts"),
            relative_path: "health.controller.ts".to_string(),
            content: Arc::from(
                r#"
@Controller('')
export class HealthController {
    @Get('/health')
    checkHealth() { }
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

        assert!(
            !result.endpoints.is_empty(),
            "Should detect NestJS route with empty prefix"
        );
        let endpoint = &result.endpoints[0];
        assert_eq!(endpoint.method, "GET");
        assert_eq!(
            endpoint.path, "/health",
            "Empty controller prefix should not prepend slash"
        );
    }

    #[test]
    fn test_nestjs_route_post() {
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
    @Post('/')
    createUser() { }
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

        assert!(
            !result.endpoints.is_empty(),
            "Should detect NestJS POST route"
        );
        let endpoint = &result.endpoints[0];
        assert_eq!(endpoint.method, "POST");
        assert_eq!(
            endpoint.path, "/users/",
            "NestJS POST route must combine prefix + method path"
        );
    }

    #[test]
    fn test_nestjs_route_with_import_statement() {
        // Mimics the exact polyglot fixture content (service-a/src/users.ts)
        let plugin = TypeScriptPlugin;

        let package_json = FileContext {
            path: std::path::PathBuf::from("/repo/service-a/package.json"),
            relative_path: "service-a/package.json".to_string(),
            content: Arc::from(
                r#"{"dependencies": {"@nestjs/core": "^10.0.0", "@nestjs/common": "^10.0.0"}}"#,
            ),
        };

        let users_file = FileContext {
            path: std::path::PathBuf::from("/repo/service-a/src/users.ts"),
            relative_path: "service-a/src/users.ts".to_string(),
            content: Arc::from(
                r#"import { Controller, Get, Post } from '@nestjs/common';

@Controller('/users')
export class UsersController {
  @Get('/:id')
  getUser(id: string) {
    return { id };
  }

  @Post('/')
  createUser() {
    return {};
  }
}"#,
            ),
        };

        let ctx = ExtractionContext {
            files: vec![package_json, users_file],
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        };

        let result = plugin.extract(&ctx);

        assert!(
            !result.endpoints.is_empty(),
            "Should detect NestJS routes from fixture content"
        );
        let get_ep = result.endpoints.iter().find(|e| e.method == "GET");
        assert!(get_ep.is_some(), "Should find GET endpoint");
        let get_ep = get_ep.unwrap();
        assert_eq!(get_ep.path, "/users/:id", "GET path must be /users/:id");
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
}
