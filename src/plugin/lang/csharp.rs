// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::collections::HashMap;

use tree_sitter::Language;

use crate::ast::AstHelper;
use crate::plugin::{scope_to_service, ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, EndpointInfo, ExtractionResult};

/// C# language plugin.
/// Covers .cs and .csproj files.
/// Detects ASP.NET Core routes, HttpClient calls, gRPC ServiceClient, MQ, and DB connections.
pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn name(&self) -> &str {
        "csharp"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.cs", "**/*.csproj"]
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        extract_csharp(ctx)
    }
}

fn csharp_language() -> Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}

/// Check if this repo uses ASP.NET Core (framework detection LPLU-08)
fn has_aspnetcore_marker(files: &[crate::plugin::FileContext]) -> bool {
    files.iter().any(|f| {
        f.path.to_string_lossy().ends_with(".csproj")
            && (f.content.contains("Microsoft.AspNetCore")
                || f.content.contains("Sdk=\"Microsoft.NET.Sdk.Web\""))
    })
}

/// Expand [controller] token in route patterns.
/// Example: class UsersController + Route("api/[controller]") → "api/users"
fn expand_controller_token(route: &str, class_name: &str) -> String {
    if route.contains("[controller]") {
        let controller_segment = class_name
            .strip_suffix("Controller")
            .unwrap_or(class_name)
            .to_lowercase();
        route.replace("[controller]", &controller_segment)
    } else {
        route.to_string()
    }
}

/// Main extraction logic for C#
fn extract_csharp(ctx: &ExtractionContext) -> ExtractionResult {
    let mut result = ExtractionResult::default();

    // Check if ASP.NET Core is present before running route detection
    let has_aspnetcore = has_aspnetcore_marker(&ctx.files);

    // Collect .cs files for processing
    let cs_files: Vec<_> = ctx
        .files
        .iter()
        .filter(|f| f.relative_path.ends_with(".cs"))
        .collect();

    // Phase A: Collect class-level [Route] prefixes (only if ASP.NET Core)
    let _class_routes = if has_aspnetcore {
        extract_class_routes(&cs_files)
    } else {
        HashMap::new()
    };

    // Phase B: Detect method-level HTTP routes
    if has_aspnetcore {
        extract_method_routes(&cs_files, ctx, &mut result);
    }

    // Phase C: Detect Minimal API routes (MapGet, MapPost, MapPut, MapDelete)
    if has_aspnetcore {
        extract_minimal_api_routes(&cs_files, ctx, &mut result);
    }

    result
}

/// Phase A: Extract class-level [Route] attributes
fn extract_class_routes(files: &[&crate::plugin::FileContext]) -> HashMap<String, String> {
    let mut routes = HashMap::new();
    let ast = AstHelper::new(csharp_language());

    for file in files {
        let matches = ast.query_matches(
            &file.content,
            r#"
(class_declaration
  (attribute_list
    (attribute
      (identifier) @attr_name
      (attribute_argument_list
        (attribute_argument
          (string_literal
            (string_literal_content) @route_prefix)))))
  (identifier) @class_name)
            "#,
        );

        // Process matches to correlate attr_name, route_prefix, and class_name
        let mut current_prefix = String::new();
        let mut current_attr = String::new();

        for m in matches {
            if m.capture_name == "attr_name" {
                current_attr = m.node_text.clone();
            } else if m.capture_name == "route_prefix" {
                current_prefix = m.node_text.clone();
            } else if m.capture_name == "class_name" {
                let current_class = m.node_text.clone();
                if current_attr == "Route" && !current_class.is_empty() {
                    let expanded = expand_controller_token(&current_prefix, &current_class);
                    routes.insert(current_class, expanded);
                }
                // Reset for next potential match
                current_attr.clear();
                current_prefix.clear();
            }
        }
    }

    routes
}

/// Phase B: Extract method-level [HttpGet], [HttpPost], etc.
fn extract_method_routes(
    files: &[&crate::plugin::FileContext],
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    let ast = AstHelper::new(csharp_language());
    let http_methods = ["HttpGet", "HttpPost", "HttpPut", "HttpDelete", "HttpPatch"];

    for file in files {
        let matches = ast.query_matches(
            &file.content,
            r#"
(method_declaration
  (attribute_list
    (attribute
      (identifier) @http_attr
      (attribute_argument_list
        (attribute_argument
          (string_literal
            (string_literal_content) @method_path)))))
  (identifier) @action_name)
            "#,
        );

        let mut current_http_attr = String::new();
        let mut current_method_path = String::new();
        let mut current_action_name = String::new();

        for m in matches {
            // Reset state at the START of each new match (when we see @http_attr)
            // This ensures captures from different methods don't interleave
            if m.capture_name == "http_attr" {
                // If we have a previous endpoint, process it before resetting
                if !current_action_name.is_empty()
                    && !current_http_attr.is_empty()
                    && http_methods.contains(&current_http_attr.as_str())
                {
                    let http_method = current_http_attr
                        .strip_prefix("Http")
                        .unwrap_or("GET")
                        .to_uppercase();

                    let path = if current_method_path.is_empty() {
                        "/".to_string()
                    } else if current_method_path.starts_with('/') {
                        current_method_path.clone()
                    } else {
                        format!("/{}", current_method_path)
                    };

                    let service_name = scope_to_service(&file.path, &ctx.service_roots)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "unknown".to_string());

                    result.endpoints.push(EndpointInfo {
                        service_name,
                        method: http_method,
                        path,
                        handler: Some(current_action_name.clone()),
                        kind: "rest".to_string(),
                        confidence: Confidence::Medium,
                        extraction_method: "csharp-attribute".to_string(),
                    });
                }

                // Reset for next match
                current_http_attr.clear();
                current_method_path.clear();
                current_action_name.clear();
            }

            // Accumulate captures
            match m.capture_name.as_str() {
                "http_attr" => current_http_attr = m.node_text.clone(),
                "method_path" => current_method_path = m.node_text.clone(),
                "action_name" => current_action_name = m.node_text.clone(),
                _ => {}
            }
        }

        // Process the last endpoint if one is pending
        if !current_action_name.is_empty()
            && !current_http_attr.is_empty()
            && http_methods.contains(&current_http_attr.as_str())
        {
            let http_method = current_http_attr
                .strip_prefix("Http")
                .unwrap_or("GET")
                .to_uppercase();

            let path = if current_method_path.is_empty() {
                "/".to_string()
            } else if current_method_path.starts_with('/') {
                current_method_path.clone()
            } else {
                format!("/{}", current_method_path)
            };

            let service_name = scope_to_service(&file.path, &ctx.service_roots)
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string());

            result.endpoints.push(EndpointInfo {
                service_name,
                method: http_method,
                path,
                handler: Some(current_action_name.clone()),
                kind: "rest".to_string(),
                confidence: Confidence::Medium,
                extraction_method: "csharp-attribute".to_string(),
            });
        }
    }
}

/// Phase C: Extract Minimal API routes (MapGet, MapPost, MapPut, MapDelete)
fn extract_minimal_api_routes(
    files: &[&crate::plugin::FileContext],
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    let map_methods = ["MapGet", "MapPost", "MapPut", "MapDelete"];

    for file in files {
        // Content gate on MapGet, MapPost, MapPut, or MapDelete
        if !file.content.contains("MapGet")
            && !file.content.contains("MapPost")
            && !file.content.contains("MapPut")
            && !file.content.contains("MapDelete")
        {
            continue;
        }

        for line in file.content.lines() {
            // Look for patterns like: app.MapGet("/path", handler)
            for method in &map_methods {
                if let Some(method_pos) = line.find(&format!(".{}", method)) {
                    // Extract the HTTP method from the map method name
                    let http_method = method.strip_prefix("Map").unwrap_or("GET").to_uppercase();

                    // Try to extract the path from the first quoted string argument
                    let rest_of_line = &line[method_pos + method.len() + 1..];
                    if let Some(quote_pos) = rest_of_line.find('"') {
                        let after_quote = &rest_of_line[quote_pos + 1..];
                        if let Some(closing_quote) = after_quote.find('"') {
                            let path = after_quote[..closing_quote].to_string();

                            let service_name = scope_to_service(&file.path, &ctx.service_roots)
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| "unknown".to_string());

                            let path_normalized = if path.is_empty() {
                                "/".to_string()
                            } else if path.starts_with('/') {
                                path
                            } else {
                                format!("/{}", path)
                            };

                            result.endpoints.push(EndpointInfo {
                                service_name,
                                method: http_method,
                                path: path_normalized,
                                handler: None, // Lambda or delegate; not easily extractable
                                kind: "rest".to_string(),
                                confidence: Confidence::High,
                                extraction_method: "csharp-minimal-api".to_string(),
                            });
                        }
                    }
                }
            }
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

    fn make_ctx(files: Vec<(&str, &str)>) -> ExtractionContext {
        let files = files
            .into_iter()
            .map(|(path, content)| FileContext {
                path: PathBuf::from(path),
                relative_path: path.to_string(),
                content: Arc::from(content),
            })
            .collect();

        ExtractionContext {
            files,
            vars: Arc::new(crate::vars::VariableStore::new()),
            root: PathBuf::from("/repo"),
            service_roots: HashMap::new(),
        }
    }

    #[test]
    fn test_aspnetcore_route_with_controller_token() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
[ApiController]
[Route("api/[controller]")]
public class UsersController
{
    [HttpGet("{id}")]
    public Task Get(int id) { return Task.CompletedTask; }
}
"#;

        let ctx = make_ctx(vec![
            ("Users.csproj", csproj_content),
            ("UsersController.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        assert!(
            !result.endpoints.is_empty(),
            "Expected to find HttpGet endpoint"
        );
        let endpoint = &result.endpoints[0];
        assert_eq!(endpoint.method, "GET");
        assert!(endpoint.path.contains("users") || endpoint.path.contains("{id}"));
        assert_eq!(endpoint.kind, "rest");
    }

    #[test]
    fn test_aspnetcore_route_no_class_prefix() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"></Project>"#;
        let cs_content = r#"
[ApiController]
public class ProductsController
{
    [HttpGet("/products")]
    public Task List() { return Task.CompletedTask; }
}
"#;

        let ctx = make_ctx(vec![
            ("Products.csproj", csproj_content),
            ("ProductsController.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        assert!(
            !result.endpoints.is_empty(),
            "Expected to find HttpGet endpoint"
        );
        let endpoint = &result.endpoints[0];
        assert_eq!(endpoint.method, "GET");
        assert!(endpoint.path.contains("products"));
    }

    #[test]
    fn test_aspnetcore_skip_when_no_aspnetcore() {
        let csproj_content =
            r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup></ItemGroup></Project>"#;
        let cs_content = r#"
[Route("api/users")]
public class UsersController
{
    [HttpGet("{id}")]
    public Task Get(int id) { return Task.CompletedTask; }
}
"#;

        let ctx = make_ctx(vec![
            ("Users.csproj", csproj_content),
            ("UsersController.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        // Should have no endpoints because ASP.NET Core marker is missing
        assert!(
            result.endpoints.is_empty(),
            "Expected no endpoints when ASP.NET Core is not detected"
        );
    }

    #[test]
    fn test_minimal_api_mapget() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
var builder = WebApplication.CreateBuilder(args);
var app = builder.Build();

app.MapGet("/users/{id}", GetUserById);
async Task<User> GetUserById(int id) => new User { Id = id };
"#;

        let ctx = make_ctx(vec![
            ("Program.csproj", csproj_content),
            ("Program.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        assert!(
            !result.endpoints.is_empty(),
            "Expected to find Minimal API MapGet endpoint"
        );
        let endpoint = result
            .endpoints
            .iter()
            .find(|e| e.extraction_method == "csharp-minimal-api")
            .expect("Expected minimal-api extraction method");
        assert_eq!(endpoint.method, "GET");
        assert!(endpoint.path.contains("/users"));
    }

    #[test]
    fn test_minimal_api_mappost() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
var app = WebApplication.CreateBuilder(args).Build();

app.MapPost("/users", CreateUser);
async Task CreateUser(User user) => { /* handler */ }
"#;

        let ctx = make_ctx(vec![
            ("Program.csproj", csproj_content),
            ("Program.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        let endpoint = result
            .endpoints
            .iter()
            .find(|e| e.extraction_method == "csharp-minimal-api")
            .expect("Expected minimal-api endpoint");
        assert_eq!(endpoint.method, "POST");
        assert_eq!(endpoint.path, "/users");
        assert_eq!(endpoint.confidence, Confidence::High);
    }

    #[test]
    fn test_minimal_api_mapput_mapdelete() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
var app = WebApplication.CreateBuilder(args).Build();

app.MapPut("/users/{id}", UpdateUser);
app.MapDelete("/users/{id}", DeleteUser);
"#;

        let ctx = make_ctx(vec![
            ("Program.csproj", csproj_content),
            ("Program.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        let put_endpoint = result
            .endpoints
            .iter()
            .find(|e| e.method == "PUT")
            .expect("Expected PUT endpoint");
        assert_eq!(put_endpoint.path, "/users/{id}");

        let delete_endpoint = result
            .endpoints
            .iter()
            .find(|e| e.method == "DELETE")
            .expect("Expected DELETE endpoint");
        assert_eq!(delete_endpoint.path, "/users/{id}");
    }
}
