// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::sync::OnceLock;

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

use crate::plugin::{scope_to_service, ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, EndpointInfo, ExtractionResult};

/// Ruby language plugin.
/// Covers .rb and Gemfile.
/// Detects: Rails routes.rb with resources expansion, Sinatra routes, Faraday/Net::HTTP clients
pub struct RubyPlugin;

/// Framework detection state
#[derive(Debug, Clone, Default)]
struct FrameworkSet {
    rails: bool,
    sinatra: bool,
    faraday: bool,
    httparty: bool,
    sidekiq: bool,
    activejob: bool,
    activerecord: bool,
}

// Rails/Sinatra literal route detection query
const ROUTE_QUERY: &str = r#"
(call
  method: (identifier) @http_verb
  arguments: (argument_list
    (string) @path
    (_)*))
"#;

// Rails resources/resource detection query
const RESOURCES_QUERY: &str = r#"
(call
  method: (identifier) @method
  arguments: (argument_list
    (simple_symbol) @resource_name
    (_)*))
"#;

// Constant.method(url, ...) — used for Faraday AND HTTParty (same AST shape, different post-filter)
// OnceLock query caches
static ROUTE_QUERY_CACHE: OnceLock<Query> = OnceLock::new();
static RESOURCES_QUERY_CACHE: OnceLock<Query> = OnceLock::new();

fn route_query(lang: &Language) -> &'static Query {
    ROUTE_QUERY_CACHE.get_or_init(|| Query::new(lang, ROUTE_QUERY).expect("valid route query"))
}

fn resources_query(lang: &Language) -> &'static Query {
    RESOURCES_QUERY_CACHE
        .get_or_init(|| Query::new(lang, RESOURCES_QUERY).expect("valid resources query"))
}

/// Detect Ruby frameworks from Gemfile in the file list
fn detect_frameworks(ctx: &ExtractionContext) -> FrameworkSet {
    let mut frameworks = FrameworkSet::default();

    for file in &ctx.files {
        if file.relative_path.ends_with("Gemfile") {
            let content = &*file.content;
            if content.contains("\"rails\"") || content.contains("'rails'") {
                frameworks.rails = true;
            }
            if content.contains("\"sinatra\"") || content.contains("'sinatra'") {
                frameworks.sinatra = true;
            }
            if content.contains("\"faraday\"") || content.contains("'faraday'") {
                frameworks.faraday = true;
            }
            if content.contains("\"httparty\"") || content.contains("'httparty'") {
                frameworks.httparty = true;
            }
            if content.contains("\"sidekiq\"") || content.contains("'sidekiq'") {
                frameworks.sidekiq = true;
            }
            if content.contains("\"activejob\"") || content.contains("'activejob'") {
                frameworks.activejob = true;
            }
            if content.contains("\"activerecord\"")
                || content.contains("'activerecord'")
                || content.contains("\"rails\"")
                || content.contains("'rails'")
            {
                frameworks.activerecord = true;
            }
        }
    }

    frameworks
}

/// Static table for Rails resources expansion
const RESOURCES_ROUTES: &[(&str, &str, &str)] = &[
    ("GET", "/{name}", "index"),
    ("POST", "/{name}", "create"),
    ("GET", "/{name}/new", "new"),
    ("GET", "/{name}/:id", "show"),
    ("GET", "/{name}/:id/edit", "edit"),
    ("PUT", "/{name}/:id", "update"),
    ("DELETE", "/{name}/:id", "destroy"),
];

/// Expand resources :name into 7 standard RESTful endpoints
fn expand_resources(resource_name: &str, confidence: Confidence) -> Vec<EndpointInfo> {
    let name = resource_name.trim_start_matches(':');

    RESOURCES_ROUTES
        .iter()
        .map(|(method, path_tmpl, _)| {
            let path = path_tmpl.replace("{name}", name);
            EndpointInfo {
                service_name: String::new(),
                method: method.to_string(),
                path,
                handler: None,
                kind: "rest".to_string(),
                confidence: confidence.clone(),
                extraction_method: "ast_rails_resources".to_string()
            }
        })
        .collect()
}

/// Expand resource :name into 6 standard RESTful endpoints (singular, no index)
fn expand_resource(resource_name: &str, confidence: Confidence) -> Vec<EndpointInfo> {
    let name = resource_name.trim_start_matches(':');

    RESOURCES_ROUTES
        .iter()
        .filter(|(_, _, action)| *action != "index")
        .map(|(method, path_tmpl, _)| {
            let path = path_tmpl.replace("{name}", name);
            EndpointInfo {
                service_name: String::new(),
                method: method.to_string(),
                path,
                handler: None,
                kind: "rest".to_string(),
                confidence: confidence.clone(),
                extraction_method: "ast_rails_resource".to_string()
            }
        })
        .collect()
}

/// Format evidence string, capping at 200 chars
impl LanguagePlugin for RubyPlugin {
    fn name(&self) -> &str {
        "ruby"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.rb", "**/Gemfile"]
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let frameworks = detect_frameworks(ctx);

        // If no Ruby frameworks detected, return empty result
        if !frameworks.rails
            && !frameworks.sinatra
            && !frameworks.faraday
            && !frameworks.httparty
            && !frameworks.sidekiq
            && !frameworks.activejob
            && !frameworks.activerecord
        {
            return ExtractionResult::default();
        }

        let lang: Language = tree_sitter_ruby::LANGUAGE.into();
        let mut parser = Parser::new();
        let _ = parser.set_language(&lang);

        let mut endpoints = Vec::new();

        // Process .rb files only (not Gemfile)
        for file in &ctx.files {
            if !file.relative_path.ends_with(".rb") {
                continue;
            }

            let source = &*file.content;
            let file_bytes = source.as_bytes();
            let is_routes_file = file.relative_path.ends_with("routes.rb");

            // Parse once per file
            let tree = match parser.parse(file_bytes, None) {
                Some(t) => t,
                None => continue,
            };

            let source_service = scope_to_service(&file.path, &ctx.service_roots);

            // Detect literal routes (get "/path", post "/path", etc.)
            // Only detect in routes.rb for Rails, or all .rb files for Sinatra
            if (frameworks.rails && is_routes_file) || (frameworks.sinatra) {
                let query = route_query(&lang);
                let mut cursor = QueryCursor::new();
                let mut matches = cursor.matches(query, tree.root_node(), file_bytes);

                while let Some(m) = matches.next() {
                    let mut http_verb = String::new();
                    let mut path = String::new();
                    let _line = 0;

                    for capture in m.captures {
                        let cap_name = query.capture_names()[capture.index as usize];
                        let text = capture
                            .node
                            .utf8_text(file_bytes)
                            .unwrap_or("")
                            .trim_matches('"')
                            .trim_matches('\'');

                        match cap_name {
                            "http_verb" => http_verb = text.to_string(),
                            "path" => path = text.to_string(),
                            _ => {}
                        }
                    }

                    if !http_verb.is_empty()
                        && !path.is_empty()
                        && ["get", "post", "put", "patch", "delete", "root", "match"]
                            .contains(&http_verb.as_str())
                    {
                        endpoints.push(EndpointInfo {
                            service_name: source_service.unwrap_or("").to_string(),
                            method: http_verb.to_uppercase(),
                            path,
                            handler: None,
                            kind: "rest".to_string(),
                            confidence: Confidence::High,
                            extraction_method: "ast_rails_route".to_string()
                        });
                    }
                }
            }

            // Detect resources and resource declarations (Rails routes.rb only)
            if frameworks.rails && is_routes_file {
                let query = resources_query(&lang);
                let mut cursor = QueryCursor::new();
                let mut matches = cursor.matches(query, tree.root_node(), file_bytes);

                while let Some(m) = matches.next() {
                    let mut method_name = String::new();
                    let mut resource_name = String::new();

                    for capture in m.captures {
                        let cap_name = query.capture_names()[capture.index as usize];
                        let text = capture
                            .node
                            .utf8_text(file_bytes)
                            .unwrap_or("")
                            .trim_matches('"')
                            .trim_matches('\'')
                            .trim_start_matches(':');

                        match cap_name {
                            "method" => method_name = text.to_string(),
                            "resource_name" => resource_name = text.to_string(),
                            _ => {}
                        }
                    }

                    // Expand resources or resource
                    if method_name == "resources" && !resource_name.is_empty() {
                        let expanded = expand_resources(&resource_name, Confidence::Medium);
                        for mut ep in expanded {
                            ep.service_name = source_service.unwrap_or("").to_string();
                            endpoints.push(ep);
                        }
                    } else if method_name == "resource" && !resource_name.is_empty() {
                        let expanded = expand_resource(&resource_name, Confidence::Medium);
                        for mut ep in expanded {
                            ep.service_name = source_service.unwrap_or("").to_string();
                            endpoints.push(ep);
                        }
                    }
                }
            }
        }

        ExtractionResult {
            services: Vec::new(),
            endpoints,
            connections: Vec::new(),
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
    fn test_rails_literal_route_without_gemfile() {
        let plugin = RubyPlugin;

        let files = vec![(
            "/repo/config/routes.rb",
            "config/routes.rb",
            r#"get "/users", to: "users#index""#,
        )];

        let ctx = create_context(files);
        let result = plugin.extract(&ctx);

        // Without Gemfile marker, framework detection will fail
        assert_eq!(result.endpoints.len(), 0);
    }

    #[test]
    fn test_rails_literal_route_with_gemfile() {
        let plugin = RubyPlugin;

        let files = vec![
            ("/repo/Gemfile", "Gemfile", r#"gem "rails", "~> 7.0""#),
            (
                "/repo/config/routes.rb",
                "config/routes.rb",
                r#"get "/users", to: "users#index""#,
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
    fn test_rails_resources_expansion() {
        let plugin = RubyPlugin;

        let files = vec![
            ("/repo/Gemfile", "Gemfile", r#"gem "rails", "~> 7.0""#),
            (
                "/repo/config/routes.rb",
                "config/routes.rb",
                r#"resources :photos"#,
            ),
        ];

        let ctx = create_context(files);
        let result = plugin.extract(&ctx);

        assert_eq!(result.endpoints.len(), 7);
        let methods: Vec<String> = result.endpoints.iter().map(|e| e.method.clone()).collect();
        assert!(methods.contains(&"GET".to_string()));
        assert!(methods.contains(&"POST".to_string()));
        assert!(methods.contains(&"PUT".to_string()));
        assert!(methods.contains(&"DELETE".to_string()));

        let paths: Vec<String> = result.endpoints.iter().map(|e| e.path.clone()).collect();
        assert!(paths.contains(&"/photos".to_string()));
        assert!(paths.contains(&"/photos/new".to_string()));
        assert!(paths.contains(&"/photos/:id".to_string()));

        for ep in &result.endpoints {
            assert_eq!(ep.confidence, Confidence::Medium);
            assert_eq!(ep.extraction_method, "ast_rails_resources");
        }
    }

    #[test]
    fn test_resources_expansion_correct_paths() {
        let expanded = expand_resources("photos", Confidence::Medium);
        assert_eq!(expanded.len(), 7);

        let paths: Vec<String> = expanded.iter().map(|e| e.path.clone()).collect();
        assert_eq!(paths[0], "/photos");
        assert_eq!(paths[1], "/photos");
        assert_eq!(paths[2], "/photos/new");
        assert_eq!(paths[3], "/photos/:id");
        assert_eq!(paths[4], "/photos/:id/edit");
        assert_eq!(paths[5], "/photos/:id");
        assert_eq!(paths[6], "/photos/:id");
    }
}
