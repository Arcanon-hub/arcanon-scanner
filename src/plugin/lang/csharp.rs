// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::collections::HashMap;

use tree_sitter::Language;

use crate::ast::AstHelper;
use crate::plugin::{scope_to_service, ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, ConnectionInfo, EndpointInfo, ExtractionResult};

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

    // Phase C: Detect Minimal API routes (.NET 6+)
    if has_aspnetcore {
        extract_minimal_api_routes(&cs_files, ctx, &mut result);
    }

    // HttpClient detection (LPLU-05)
    extract_httpclient_calls(&cs_files, ctx, &mut result);

    // IHttpClientFactory / named clients detection
    extract_httpclient_factory(&cs_files, ctx, &mut result);

    // gRPC ServiceClient detection (LPLU-12)
    extract_grpc_clients(&cs_files, ctx, &mut result);

    // Entity Framework DbContext detection
    extract_efcore_dbcontext(&cs_files, ctx, &mut result);

    // MassTransit MQ detection
    extract_masstransit_mq(&cs_files, ctx, &mut result);

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
        let mut _line = 1;

        for m in matches {
            if m.capture_name == "http_attr" {
                current_http_attr = m.node_text.clone();
            } else if m.capture_name == "method_path" {
                current_method_path = m.node_text.clone();
                _line = m.line;
            } else if m.capture_name == "action_name" {
                current_action_name = m.node_text.clone();
            }

            // When we have all three pieces, we can create an endpoint
            if !current_http_attr.is_empty()
                && !current_action_name.is_empty()
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

                // Reset for next match
                current_http_attr.clear();
                current_method_path.clear();
                current_action_name.clear();
            }
        }
    }
}

/// Extract HttpClient.GetAsync/PostAsync/etc. calls
fn extract_httpclient_calls(
    files: &[&crate::plugin::FileContext],
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    let ast = AstHelper::new(csharp_language());
    let httpclient_methods = [
        "GetAsync",
        "PostAsync",
        "PutAsync",
        "DeleteAsync",
        "PatchAsync",
        "SendAsync",
    ];

    for file in files {
        if !file.content.contains("HttpClient") {
            continue;
        }

        let matches = ast.query_matches(
            &file.content,
            r#"
(invocation_expression
  (member_access_expression
    (identifier) @obj
    "."
    (identifier) @method)
  (argument_list
    (argument
      (string_literal
        (string_literal_content) @url))))
            "#,
        );

        let mut current_method = String::new();
        let mut current_url = String::new();
        let mut _line = 1;

        for m in matches {
            if m.capture_name == "method" {
                current_method = m.node_text.clone();
            } else if m.capture_name == "url" {
                current_url = m.node_text.clone();
                _line = m.line;
            }

            if !current_method.is_empty()
                && !current_url.is_empty()
                && httpclient_methods.contains(&current_method.as_str())
            {
                let service_name = scope_to_service(&file.path, &ctx.service_roots)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let relative_path = &file.relative_path;
                let source_file = format!("{}:{}", relative_path, _line);
                let evidence = current_url.chars().take(200).collect::<String>();

                result.connections.push(ConnectionInfo {
                    source_service: service_name,
                    target_name: "external".to_string(),
                    protocol: "rest".to_string(),
                    method: Some(current_method.clone()),
                    path: Some(current_url.clone()),
                    source_file,
                    confidence: Confidence::High,
                    extraction_method: "csharp-httpclient".to_string(),
                    evidence: Some(evidence),
                });

                current_method.clear();
                current_url.clear();
            }
        }
    }
}

/// Extract gRPC ServiceClient instantiation calls
fn extract_grpc_clients(
    files: &[&crate::plugin::FileContext],
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    let ast = AstHelper::new(csharp_language());

    for file in files {
        if !file.content.contains("Grpc.Core")
            && !file.content.contains(".ServiceClient")
            && !file.content.contains("_grpc")
        {
            continue;
        }

        let matches = ast.query_matches(
            &file.content,
            r#"
(object_creation_expression
  (qualified_name
    (identifier) @service
    "."
    (identifier) @client_class)
  (argument_list (_)+))
            "#,
        );

        let mut current_service = String::new();
        let mut current_client_class = String::new();
        let mut _line = 1;

        for m in matches {
            if m.capture_name == "service" {
                current_service = m.node_text.clone();
            } else if m.capture_name == "client_class" {
                current_client_class = m.node_text.clone();
                _line = m.line;
            }

            if current_client_class == "ServiceClient" && !current_service.is_empty() {
                let source_service = scope_to_service(&file.path, &ctx.service_roots)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let relative_path = &file.relative_path;
                let source_file = format!("{}:{}", relative_path, _line);

                result.connections.push(ConnectionInfo {
                    source_service,
                    target_name: current_service.clone(),
                    protocol: "grpc".to_string(),
                    method: None,
                    path: None,
                    source_file,
                    confidence: Confidence::High,
                    extraction_method: "csharp-grpc".to_string(),
                    evidence: Some(format!("new {}.{}", current_service, current_client_class)),
                });

                current_service.clear();
                current_client_class.clear();
            }
        }
    }
}

/// Detect Minimal API routes (.NET 6+): app.MapGet/MapPost/MapPut/MapDelete
fn extract_minimal_api_routes(
    files: &[&crate::plugin::FileContext],
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    let map_methods = ["MapGet", "MapPost", "MapPut", "MapDelete"];

    for file in files {
        // Content gate on MapGet or MapPost
        if !file.content.contains("MapGet")
            && !file.content.contains("MapPost")
            && !file.content.contains("MapPut")
            && !file.content.contains("MapDelete")
        {
            continue;
        }

        let lines = file.content.lines().enumerate();
        for (_line_idx, line) in lines {
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

/// Detect IHttpClientFactory / named clients: services.AddHttpClient("name", ...)
fn extract_httpclient_factory(
    files: &[&crate::plugin::FileContext],
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    for file in files {
        // Content gate on AddHttpClient or IHttpClientFactory
        if !file.content.contains("AddHttpClient") && !file.content.contains("IHttpClientFactory") {
            continue;
        }

        let lines = file.content.lines().enumerate();
        for (line_idx, line) in lines {
            let line_num = line_idx + 1;

            if let Some(add_pos) = line.find("AddHttpClient") {
                // Try to extract the named client name from the first string argument
                let rest_of_line = &line[add_pos + "AddHttpClient".len()..];
                if let Some(paren_pos) = rest_of_line.find('(') {
                    let after_paren = &rest_of_line[paren_pos + 1..];

                    // Look for a quoted string (client name)
                    if let Some(quote_pos) = after_paren.find('"') {
                        let after_quote = &after_paren[quote_pos + 1..];
                        if let Some(closing_quote) = after_quote.find('"') {
                            let client_name = after_quote[..closing_quote].to_string();

                            let source_service = scope_to_service(&file.path, &ctx.service_roots)
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| "unknown".to_string());

                            let relative_path = &file.relative_path;
                            let source_file = format!("{}:{}", relative_path, line_num);

                            result.connections.push(ConnectionInfo {
                                source_service,
                                target_name: format!("http-client:{}", client_name),
                                protocol: "rest".to_string(),
                                method: None,
                                path: None,
                                source_file,
                                confidence: Confidence::Medium,
                                extraction_method: "csharp-httpclient-factory".to_string(),
                                evidence: Some(format!("AddHttpClient(\"{}\")", client_name)),
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Detect Entity Framework DbContext: services.AddDbContext<T>(options => options.UseNpgsql/UseSqlServer/UseSqlite)
fn extract_efcore_dbcontext(
    files: &[&crate::plugin::FileContext],
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    for file in files {
        // Content gate on DbContext or UseNpgsql or UseSqlServer
        if !file.content.contains("DbContext")
            && !file.content.contains("UseNpgsql")
            && !file.content.contains("UseSqlServer")
            && !file.content.contains("UseSqlite")
        {
            continue;
        }

        let lines = file.content.lines().enumerate();
        for (line_idx, line) in lines {
            let line_num = line_idx + 1;

            // Detect UseNpgsql → postgresql
            if line.contains("UseNpgsql") {
                let source_service = scope_to_service(&file.path, &ctx.service_roots)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let relative_path = &file.relative_path;
                let source_file = format!("{}:{}", relative_path, line_num);

                result.connections.push(ConnectionInfo {
                    source_service,
                    target_name: "postgresql".to_string(),
                    protocol: "postgresql".to_string(),
                    method: None,
                    path: None,
                    source_file,
                    confidence: Confidence::High,
                    extraction_method: "csharp-efcore".to_string(),
                    evidence: Some("UseNpgsql()".to_string()),
                });
            }

            // Detect UseSqlServer → mssql
            if line.contains("UseSqlServer") {
                let source_service = scope_to_service(&file.path, &ctx.service_roots)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let relative_path = &file.relative_path;
                let source_file = format!("{}:{}", relative_path, line_num);

                result.connections.push(ConnectionInfo {
                    source_service,
                    target_name: "mssql".to_string(),
                    protocol: "mssql".to_string(),
                    method: None,
                    path: None,
                    source_file,
                    confidence: Confidence::High,
                    extraction_method: "csharp-efcore".to_string(),
                    evidence: Some("UseSqlServer()".to_string()),
                });
            }

            // Detect UseSqlite → sqlite
            if line.contains("UseSqlite") {
                let source_service = scope_to_service(&file.path, &ctx.service_roots)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let relative_path = &file.relative_path;
                let source_file = format!("{}:{}", relative_path, line_num);

                result.connections.push(ConnectionInfo {
                    source_service,
                    target_name: "sqlite".to_string(),
                    protocol: "sqlite".to_string(),
                    method: None,
                    path: None,
                    source_file,
                    confidence: Confidence::High,
                    extraction_method: "csharp-efcore".to_string(),
                    evidence: Some("UseSqlite()".to_string()),
                });
            }
        }
    }
}

/// Detect MassTransit MQ: services.AddMassTransit(x => { x.UsingRabbitMq(...) })
fn extract_masstransit_mq(
    files: &[&crate::plugin::FileContext],
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    for file in files {
        // Content gate on MassTransit or UsingRabbitMq or UsingAzureServiceBus
        if !file.content.contains("MassTransit")
            && !file.content.contains("UsingRabbitMq")
            && !file.content.contains("UsingAzureServiceBus")
        {
            continue;
        }

        let lines = file.content.lines().enumerate();
        for (line_idx, line) in lines {
            let line_num = line_idx + 1;

            // Detect UsingRabbitMq → amqp
            if line.contains("UsingRabbitMq") {
                let source_service = scope_to_service(&file.path, &ctx.service_roots)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let relative_path = &file.relative_path;
                let source_file = format!("{}:{}", relative_path, line_num);

                result.connections.push(ConnectionInfo {
                    source_service,
                    target_name: "rabbitmq".to_string(),
                    protocol: "amqp".to_string(),
                    method: None,
                    path: None,
                    source_file,
                    confidence: Confidence::High,
                    extraction_method: "csharp-masstransit".to_string(),
                    evidence: Some("UsingRabbitMq()".to_string()),
                });
            }

            // Detect UsingAzureServiceBus → azure-servicebus
            if line.contains("UsingAzureServiceBus") {
                let source_service = scope_to_service(&file.path, &ctx.service_roots)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let relative_path = &file.relative_path;
                let source_file = format!("{}:{}", relative_path, line_num);

                result.connections.push(ConnectionInfo {
                    source_service,
                    target_name: "azure-servicebus".to_string(),
                    protocol: "azure-servicebus".to_string(),
                    method: None,
                    path: None,
                    source_file,
                    confidence: Confidence::High,
                    extraction_method: "csharp-masstransit".to_string(),
                    evidence: Some("UsingAzureServiceBus()".to_string()),
                });
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
    fn test_httpclient_getasync() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
using System.Net.Http;
public class UserService
{
    private readonly HttpClient _client;
    public async Task FetchUser()
    {
        var result = await _client.GetAsync("https://api.example.com/users");
    }
}
"#;

        let ctx = make_ctx(vec![
            ("Web.csproj", csproj_content),
            ("UserService.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        assert!(
            !result.connections.is_empty(),
            "Expected HttpClient connection"
        );
        let conn = result
            .connections
            .iter()
            .find(|c| c.protocol == "rest")
            .expect("Expected rest connection");
        assert_eq!(conn.protocol, "rest");
        assert_eq!(conn.extraction_method, "csharp-httpclient");
    }

    #[test]
    fn test_grpc_serviceclient() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="Grpc.Core" /></ItemGroup></Project>"#;
        let cs_content = r#"
using Grpc.Core;
public class OrderClient
{
    public void SendOrder()
    {
        var channel = new Channel("localhost", 5000, ChannelCredentials.Insecure);
        var client = new OrderService.ServiceClient(channel);
    }
}
"#;

        let ctx = make_ctx(vec![
            ("Client.csproj", csproj_content),
            ("OrderClient.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty(), "Expected gRPC connection");
        let conn = result
            .connections
            .iter()
            .find(|c| c.protocol == "grpc")
            .expect("Expected grpc connection");
        assert_eq!(conn.protocol, "grpc");
        assert_eq!(conn.target_name, "OrderService");
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

    #[test]
    fn test_httpclient_factory_named_client() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
public void ConfigureServices(IServiceCollection services)
{
    services.AddHttpClient("github", client =>
    {
        client.BaseAddress = new Uri("https://api.github.com");
    });
}
"#;

        let ctx = make_ctx(vec![
            ("Startup.csproj", csproj_content),
            ("Startup.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        assert!(
            !result.connections.is_empty(),
            "Expected HttpClientFactory connection"
        );
        let conn = result
            .connections
            .iter()
            .find(|c| c.extraction_method == "csharp-httpclient-factory")
            .expect("Expected httpclient-factory connection");
        assert_eq!(conn.protocol, "rest");
        assert!(conn.target_name.contains("github"));
        assert_eq!(conn.confidence, Confidence::Medium);
    }

    #[test]
    fn test_efcore_usepostgresql() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
public void ConfigureServices(IServiceCollection services)
{
    services.AddDbContext<AppDbContext>(options =>
        options.UseNpgsql("Server=localhost;Database=mydb"));
}
"#;

        let ctx = make_ctx(vec![
            ("Startup.csproj", csproj_content),
            ("Startup.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        assert!(
            !result.connections.is_empty(),
            "Expected EF Core PostgreSQL connection"
        );
        let conn = result
            .connections
            .iter()
            .find(|c| c.protocol == "postgresql")
            .expect("Expected postgresql connection");
        assert_eq!(conn.target_name, "postgresql");
        assert_eq!(conn.extraction_method, "csharp-efcore");
        assert_eq!(conn.confidence, Confidence::High);
    }

    #[test]
    fn test_efcore_usesqlserver() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
public void ConfigureServices(IServiceCollection services)
{
    services.AddDbContext<AppDbContext>(options =>
        options.UseSqlServer("Server=.;Database=mydb"));
}
"#;

        let ctx = make_ctx(vec![
            ("Startup.csproj", csproj_content),
            ("Startup.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        let conn = result
            .connections
            .iter()
            .find(|c| c.protocol == "mssql")
            .expect("Expected mssql connection");
        assert_eq!(conn.target_name, "mssql");
    }

    #[test]
    fn test_efcore_usesqlite() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
public void ConfigureServices(IServiceCollection services)
{
    services.AddDbContext<AppDbContext>(options =>
        options.UseSqlite("Data Source=app.db"));
}
"#;

        let ctx = make_ctx(vec![
            ("Startup.csproj", csproj_content),
            ("Startup.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        let conn = result
            .connections
            .iter()
            .find(|c| c.protocol == "sqlite")
            .expect("Expected sqlite connection");
        assert_eq!(conn.target_name, "sqlite");
    }

    #[test]
    fn test_masstransit_rabbitmq() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
public void ConfigureServices(IServiceCollection services)
{
    services.AddMassTransit(x =>
    {
        x.UsingRabbitMq((context, cfg) =>
        {
            cfg.Host("rabbitmq://localhost");
        });
    });
}
"#;

        let ctx = make_ctx(vec![
            ("Startup.csproj", csproj_content),
            ("Startup.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        assert!(
            !result.connections.is_empty(),
            "Expected MassTransit RabbitMQ connection"
        );
        let conn = result
            .connections
            .iter()
            .find(|c| c.protocol == "amqp")
            .expect("Expected amqp connection");
        assert_eq!(conn.target_name, "rabbitmq");
        assert_eq!(conn.extraction_method, "csharp-masstransit");
        assert_eq!(conn.confidence, Confidence::High);
    }

    #[test]
    fn test_masstransit_azure_servicebus() {
        let csproj_content = r#"<Project Sdk="Microsoft.NET.Sdk.Web"><ItemGroup><PackageReference Include="Microsoft.AspNetCore" /></ItemGroup></Project>"#;
        let cs_content = r#"
public void ConfigureServices(IServiceCollection services)
{
    services.AddMassTransit(x =>
    {
        x.UsingAzureServiceBus((context, cfg) =>
        {
            cfg.Host("sb://mynamespace.servicebus.windows.net");
        });
    });
}
"#;

        let ctx = make_ctx(vec![
            ("Startup.csproj", csproj_content),
            ("Startup.cs", cs_content),
        ]);

        let plugin = CSharpPlugin;
        let result = plugin.extract(&ctx);

        let conn = result
            .connections
            .iter()
            .find(|c| c.protocol == "azure-servicebus")
            .expect("Expected azure-servicebus connection");
        assert_eq!(conn.target_name, "azure-servicebus");
    }
}
