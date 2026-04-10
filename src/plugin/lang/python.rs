// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::sync::OnceLock;
use tree_sitter::{Parser, Query, QueryCursor, StreamingIterator};

use crate::plugin::{scope_to_service, ExtractionContext, FileContext, LanguagePlugin};
use crate::types::{Confidence, EndpointInfo, ExtractionResult};

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
                    ..Default::default()
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
                    ..Default::default()
                });
            }
        }
    }

    endpoints
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
}
