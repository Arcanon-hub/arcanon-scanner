// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use std::collections::HashMap;
use tree_sitter::Parser;

use crate::plugin::{scope_to_service, ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, EndpointInfo, ExtractionResult};

/// Java language plugin.
/// Covers .java, pom.xml, and build.gradle.
pub struct JavaPlugin;

impl LanguagePlugin for JavaPlugin {
    fn name(&self) -> &str {
        "java"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.java", "**/pom.xml", "**/build.gradle"]
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        let has_spring = detect_spring_framework(ctx);

        for file in ctx.files.iter() {
            if file.relative_path.ends_with(".java") {
                if has_spring {
                    extract_routes_from_file(&file.content, &file.relative_path, ctx, &mut result);
                }
                extract_connections_from_file(&file.content, &file.relative_path, ctx, &mut result);
            }
        }

        result
    }
}

fn detect_spring_framework(ctx: &ExtractionContext) -> bool {
    for file in &ctx.files {
        if file.relative_path.ends_with("pom.xml") {
            if file.content.contains("spring-boot-starter-web")
                || file.content.contains("spring-boot-starter")
            {
                return true;
            }
        } else if file.relative_path.ends_with("build.gradle")
            && file.content.contains("spring-boot-starter-web") {
                return true;
            }
    }
    false
}

fn extract_routes_from_file(
    content: &str,
    relative_path: &str,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_java::LANGUAGE.into()).is_err() {
        return;
    }

    let tree = match parser.parse(content.as_bytes(), None) {
        Some(t) => t,
        None => return,
    };

    let mut class_prefixes: HashMap<String, String> = HashMap::new();
    extract_class_prefixes(tree.root_node(), content, &mut class_prefixes);

    extract_method_routes(
        tree.root_node(),
        content,
        &class_prefixes,
        relative_path,
        ctx,
        result,
    );
}

fn extract_class_prefixes(
    node: tree_sitter::Node,
    source: &str,
    class_prefixes: &mut HashMap<String, String>,
) {
    if node.kind() == "class_declaration" {
        let mut class_name = String::new();
        let mut class_prefix = String::new();

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "modifiers" => {
                    extract_prefix_from_modifiers(child, source, &mut class_prefix);
                }
                "identifier" if class_name.is_empty() => {
                    if let Ok(name) = child.utf8_text(source.as_bytes()) {
                        class_name = name.to_string();
                    }
                }
                _ => {}
            }
        }

        if !class_name.is_empty() {
            class_prefixes.insert(class_name, class_prefix);
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_class_prefixes(child, source, class_prefixes);
    }
}

fn extract_prefix_from_modifiers(node: tree_sitter::Node, source: &str, prefix: &mut String) {
    let mut cursor = node.walk();
    for annotation in node.children(&mut cursor) {
        if annotation.kind() == "annotation"
            && is_route_annotation(annotation, source) {
                extract_path_from_annotation(annotation, source, prefix);
            }
    }
}

fn is_route_annotation(node: tree_sitter::Node, source: &str) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" {
            if let Ok(name) = child.utf8_text(source.as_bytes()) {
                return matches!(name, "RestController" | "Controller" | "RequestMapping");
            }
        }
    }
    false
}

fn extract_path_from_annotation(node: tree_sitter::Node, source: &str, path: &mut String) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "annotation_argument_list" {
            extract_path_from_arg_list(child, source, path);
        }
    }
}

fn extract_path_from_arg_list(node: tree_sitter::Node, source: &str, path: &mut String) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "string_literal" => {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    *path = text.trim_matches('"').to_string();
                }
            }
            "element_value_pair" => {
                let mut pair_cursor = child.walk();
                let mut key = String::new();
                for pair_child in child.children(&mut pair_cursor) {
                    if pair_child.kind() == "identifier" && key.is_empty() {
                        if let Ok(k) = pair_child.utf8_text(source.as_bytes()) {
                            key = k.to_string();
                        }
                    } else if pair_child.kind() == "string_literal"
                        && matches!(key.as_str(), "value" | "path") {
                            if let Ok(v) = pair_child.utf8_text(source.as_bytes()) {
                                *path = v.trim_matches('"').to_string();
                            }
                        }
                }
            }
            _ => {}
        }
    }
}

fn extract_method_routes(
    node: tree_sitter::Node,
    source: &str,
    class_prefixes: &HashMap<String, String>,
    relative_path: &str,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    if node.kind() == "method_declaration" {
        process_method_node(node, source, class_prefixes, relative_path, ctx, result);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_method_routes(child, source, class_prefixes, relative_path, ctx, result);
    }
}

fn process_method_node(
    method_node: tree_sitter::Node,
    source: &str,
    class_prefixes: &HashMap<String, String>,
    relative_path: &str,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    let mut http_method = String::new();
    let mut method_path = String::new();

    let mut cursor = method_node.walk();
    for child in method_node.children(&mut cursor) {
        if child.kind() == "modifiers" {
            extract_http_method(child, source, &mut http_method, &mut method_path);
        }
    }

    if http_method.is_empty() {
        return;
    }

    let class_name = find_enclosing_class_name(method_node, source);
    let class_prefix = class_prefixes.get(&class_name).cloned().unwrap_or_default();
    let full_path = format!("{}{}", class_prefix, method_path);

    let service_name = scope_to_service(&ctx.root.join(relative_path), &ctx.service_roots)
        .unwrap_or("unknown")
        .to_string();

    result.endpoints.push(EndpointInfo {
        service_name,
        method: http_method,
        path: full_path,
        handler: None,
        kind: "rest".to_string(),
        confidence: Confidence::High,
        extraction_method: "java_spring_boot".to_string(),
    });
}

fn extract_http_method(
    modifiers: tree_sitter::Node,
    source: &str,
    http_method: &mut String,
    path: &mut String,
) {
    let mut cursor = modifiers.walk();
    for annotation in modifiers.children(&mut cursor) {
        if annotation.kind() == "annotation" {
            let mut ann_name = String::new();
            let mut ann_cursor = annotation.walk();
            for child in annotation.children(&mut ann_cursor) {
                if child.kind() == "identifier" {
                    if let Ok(name) = child.utf8_text(source.as_bytes()) {
                        ann_name = name.to_string();
                    }
                    break;
                }
            }

            let verb = match ann_name.as_str() {
                "GetMapping" => "GET",
                "PostMapping" => "POST",
                "PutMapping" => "PUT",
                "DeleteMapping" => "DELETE",
                "PatchMapping" => "PATCH",
                "RequestMapping" => "GET",
                _ => continue,
            };

            *http_method = verb.to_string();

            let mut ann_cursor = annotation.walk();
            for child in annotation.children(&mut ann_cursor) {
                if child.kind() == "annotation_argument_list" {
                    extract_path_from_arg_list(child, source, path);
                }
            }
            return;
        }
    }
}

fn find_enclosing_class_name(method_node: tree_sitter::Node, source: &str) -> String {
    let mut current = method_node.parent();
    while let Some(node) = current {
        if node.kind() == "class_body" {
            if let Some(class_decl) = node.parent() {
                if class_decl.kind() == "class_declaration" {
                    let mut cd_cursor = class_decl.walk();
                    for child in class_decl.children(&mut cd_cursor) {
                        if child.kind() == "identifier" {
                            if let Ok(name) = child.utf8_text(source.as_bytes()) {
                                return name.to_string();
                            }
                        }
                    }
                }
            }
        }
        current = node.parent();
    }
    String::new()
}


fn extract_connections_from_file(
    content: &str,
    relative_path: &str,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    // RestTemplate detection
    if content.contains("RestTemplate") || content.contains("restTemplate") {
        detect_rest_template(content, relative_path, ctx, result);
    }

    // gRPC detection
    if content.contains("Grpc.") {
        detect_grpc(content, relative_path, ctx, result);
    }

    // RabbitMQ detection
    if content.contains("RabbitTemplate") || content.contains("AmqpTemplate") {
        detect_rabbit_mq(content, relative_path, ctx, result);
    }

    // Kafka detection
    if content.contains("KafkaTemplate") {
        detect_kafka(content, relative_path, ctx, result);
    }

    // JDBC detection
    if content.contains("JdbcTemplate") {
        detect_jdbc(content, relative_path, ctx, result);
    }
}

fn detect_rest_template(
    content: &str,
    relative_path: &str,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    // Simple pattern matching for RestTemplate method calls
    let methods = ["getForObject", "postForObject", "exchange", "getForEntity"];
    for method in &methods {
        if content.contains(method) {
            let service_name = scope_to_service(&ctx.root.join(relative_path), &ctx.service_roots)
                .unwrap_or("unknown")
                .to_string();

            result.connections.push(crate::types::ConnectionInfo {
                source_service: service_name,
                target_name: "unknown".to_string(),
                protocol: "rest".to_string(),
                method: None,
                path: None,
                source_file: format!("{}:1", relative_path),
                confidence: Confidence::Medium,
                extraction_method: "java_rest_template".to_string(),
                evidence: Some(format!("RestTemplate.{} call detected", method)),
            });
            break;
        }
    }
}

fn detect_grpc(
    content: &str,
    relative_path: &str,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    // Look for ServiceGrpc.newBlockingStub pattern
    if let Some(start) = content.find("Grpc.new") {
        if let Some(class_end) = content[..start].rfind(' ') {
            let class_part = &content[class_end + 1..start];
            if class_part.contains("Service") {
                let target_name = class_part.replace("Service", "");
                let service_name = scope_to_service(&ctx.root.join(relative_path), &ctx.service_roots)
                    .unwrap_or("unknown")
                    .to_string();

                result.connections.push(crate::types::ConnectionInfo {
                    source_service: service_name,
                    target_name,
                    protocol: "grpc".to_string(),
                    method: None,
                    path: None,
                    source_file: format!("{}:1", relative_path),
                    confidence: Confidence::High,
                    extraction_method: "java_grpc".to_string(),
                    evidence: Some(format!("{}Grpc.newBlockingStub() call", class_part)),
                });
            }
        }
    }
}

fn detect_rabbit_mq(
    content: &str,
    relative_path: &str,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    if content.contains("convertAndSend") || content.contains("basicPublish") {
        let service_name = scope_to_service(&ctx.root.join(relative_path), &ctx.service_roots)
            .unwrap_or("unknown")
            .to_string();

        result.connections.push(crate::types::ConnectionInfo {
            source_service: service_name,
            target_name: "rabbitmq".to_string(),
            protocol: "amqp".to_string(),
            method: None,
            path: None,
            source_file: format!("{}:1", relative_path),
            confidence: Confidence::High,
            extraction_method: "java_rabbit_mq".to_string(),
            evidence: Some("RabbitTemplate message publish detected".to_string()),
        });
    }
}

fn detect_kafka(
    content: &str,
    relative_path: &str,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    if content.contains("kafkaTemplate.send") {
        let service_name = scope_to_service(&ctx.root.join(relative_path), &ctx.service_roots)
            .unwrap_or("unknown")
            .to_string();

        result.connections.push(crate::types::ConnectionInfo {
            source_service: service_name,
            target_name: "kafka".to_string(),
            protocol: "kafka".to_string(),
            method: None,
            path: None,
            source_file: format!("{}:1", relative_path),
            confidence: Confidence::High,
            extraction_method: "java_kafka".to_string(),
            evidence: Some("KafkaTemplate.send() call detected".to_string()),
        });
    }
}

fn detect_jdbc(
    content: &str,
    relative_path: &str,
    ctx: &ExtractionContext,
    result: &mut ExtractionResult,
) {
    if content.contains("JdbcTemplate") {
        let service_name = scope_to_service(&ctx.root.join(relative_path), &ctx.service_roots)
            .unwrap_or("unknown")
            .to_string();

        result.connections.push(crate::types::ConnectionInfo {
            source_service: service_name,
            target_name: "database".to_string(),
            protocol: "postgresql".to_string(),
            method: None,
            path: None,
            source_file: format!("{}:1", relative_path),
            confidence: Confidence::Medium,
            extraction_method: "java_jdbc".to_string(),
            evidence: Some("JdbcTemplate usage detected".to_string()),
        });
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_context(files: Vec<(String, String)>) -> ExtractionContext {
        let ctx_files = files
            .into_iter()
            .map(|(path, content)| crate::plugin::FileContext {
                path: std::path::PathBuf::from(&path),
                relative_path: path,
                content: std::sync::Arc::from(content),
            })
            .collect();

        ExtractionContext {
            files: ctx_files,
            vars: std::sync::Arc::new(crate::vars::VariableStore::new()),
            root: std::path::PathBuf::from("/test"),
            service_roots: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn test_spring_marker_detection() {
        let ctx = create_test_context(vec![(
            "pom.xml".to_string(),
            "<dependency><artifactId>spring-boot-starter-web</artifactId></dependency>".to_string(),
        )]);
        assert!(detect_spring_framework(&ctx));
    }

    #[test]
    fn test_request_mapping_with_get_mapping() {
        let java_code = r#"
@RestController
@RequestMapping("/api/v1")
public class OrdersController {
    @GetMapping("/orders")
    public void listOrders() {}
}
"#;
        let ctx = create_test_context(vec![
            (
                "pom.xml".to_string(),
                "<dependency><artifactId>spring-boot-starter-web</artifactId></dependency>".to_string(),
            ),
            ("src/OrdersController.java".to_string(), java_code.to_string()),
        ]);

        let plugin = JavaPlugin;
        let result = plugin.extract(&ctx);

        assert!(!result.endpoints.is_empty());
        assert_eq!(result.endpoints[0].method, "GET");
        assert_eq!(result.endpoints[0].path, "/api/v1/orders");
    }

    #[test]
    fn test_get_mapping_without_prefix() {
        let java_code = r#"
@RestController
public class UserController {
    @GetMapping("/users")
    public void listUsers() {}
}
"#;
        let ctx = create_test_context(vec![
            (
                "pom.xml".to_string(),
                "<dependency><artifactId>spring-boot-starter-web</artifactId></dependency>".to_string(),
            ),
            ("src/UserController.java".to_string(), java_code.to_string()),
        ]);

        let plugin = JavaPlugin;
        let result = plugin.extract(&ctx);

        assert!(!result.endpoints.is_empty());
        assert_eq!(result.endpoints[0].method, "GET");
        assert_eq!(result.endpoints[0].path, "/users");
    }

    #[test]
    fn test_no_routes_without_spring() {
        let java_code = r#"
@RestController
public class UserController {
    @GetMapping("/users")
    public void listUsers() {}
}
"#;
        let ctx = create_test_context(vec![("src/UserController.java".to_string(), java_code.to_string())]);

        let plugin = JavaPlugin;
        let result = plugin.extract(&ctx);

        assert!(result.endpoints.is_empty());
    }

    #[test]
    fn test_rest_template_detection() {
        let java_code = r#"
public class OrderService {
    @Autowired
    private RestTemplate restTemplate;

    public void fetchOrder(String id) {
        Order order = restTemplate.getForObject("/api/orders/" + id, Order.class);
    }
}
"#;
        let ctx = create_test_context(vec![("src/OrderService.java".to_string(), java_code.to_string())]);

        let plugin = JavaPlugin;
        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty());
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "rest");
        assert_eq!(conn.extraction_method, "java_rest_template");
    }

    #[test]
    fn test_grpc_stub_detection() {
        let java_code = r#"
public class OrderClient {
    private OrderServiceGrpc.OrderServiceBlockingStub stub;

    public Order getOrder(String id) {
        this.stub = OrderServiceGrpc.newBlockingStub(channel);
        return stub.getOrder(GetOrderRequest.newBuilder().setId(id).build());
    }
}
"#;
        let ctx = create_test_context(vec![("src/OrderClient.java".to_string(), java_code.to_string())]);

        let plugin = JavaPlugin;
        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty());
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "grpc");
        assert_eq!(conn.confidence, Confidence::High);
    }

    #[test]
    fn test_rabbit_mq_detection() {
        let java_code = r#"
public class OrderPublisher {
    @Autowired
    private RabbitTemplate rabbitTemplate;

    public void publishOrder(Order order) {
        rabbitTemplate.convertAndSend("orders", order);
    }
}
"#;
        let ctx = create_test_context(vec![("src/OrderPublisher.java".to_string(), java_code.to_string())]);

        let plugin = JavaPlugin;
        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty());
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "amqp");
        assert_eq!(conn.target_name, "rabbitmq");
    }

    #[test]
    fn test_kafka_detection() {
        let java_code = r#"
public class EventPublisher {
    @Autowired
    private KafkaTemplate<String, String> kafkaTemplate;

    public void publish(String topic, String message) {
        kafkaTemplate.send(topic, message);
    }
}
"#;
        let ctx = create_test_context(vec![("src/EventPublisher.java".to_string(), java_code.to_string())]);

        let plugin = JavaPlugin;
        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty());
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "kafka");
    }

    #[test]
    fn test_jdbc_detection() {
        let java_code = r#"
public class OrderRepository {
    @Autowired
    private JdbcTemplate jdbcTemplate;

    public Order findById(String id) {
        return jdbcTemplate.queryForObject("SELECT * FROM orders WHERE id = ?", Order.class, id);
    }
}
"#;
        let ctx = create_test_context(vec![("src/OrderRepository.java".to_string(), java_code.to_string())]);

        let plugin = JavaPlugin;
        let result = plugin.extract(&ctx);

        assert!(!result.connections.is_empty());
        let conn = &result.connections[0];
        assert_eq!(conn.protocol, "postgresql");
        assert_eq!(conn.extraction_method, "java_jdbc");
    }
}
