use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::{Confidence, EndpointInfo, ExtractionResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::warn;

/// AsyncAPI specification parser (no asyncapi crate; custom serde structs).
pub struct AsyncApiPlugin;

impl LanguagePlugin for AsyncApiPlugin {
    fn name(&self) -> &str {
        "asyncapi"
    }

    fn file_patterns(&self) -> &[&str] {
        &[
            "**/asyncapi.json",
            "**/asyncapi.yaml",
            "**/asyncapi.yml",
            "**/*.asyncapi.json",
            "**/*.asyncapi.yaml",
            "**/*.asyncapi.yml",
        ]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, ctx: &ExtractionContext) -> ExtractionResult {
        let mut result = ExtractionResult::default();

        for file in &ctx.files {
            match parse_asyncapi(&file.content, &file.relative_path) {
                Ok(endpoints) => {
                    result.endpoints.extend(endpoints);
                }
                Err(e) => {
                    warn!("asyncapi: failed to parse {}: {}", file.relative_path, e);
                }
            }
        }

        result
    }
}

/// AsyncAPI 2.x specification structs (custom, no external asyncapi crate)
#[derive(Debug, Deserialize, Serialize, Default)]
struct AsyncApiSpec {
    asyncapi: Option<String>,
    info: Option<AsyncApiInfo>,
    channels: Option<HashMap<String, AsyncApiChannel>>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct AsyncApiInfo {
    title: Option<String>,
    version: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct AsyncApiChannel {
    #[serde(default)]
    subscribe: Option<AsyncApiOperation>,
    #[serde(default)]
    publish: Option<AsyncApiOperation>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct AsyncApiOperation {
    #[serde(rename = "operationId")]
    operation_id: Option<String>,
    message: Option<serde_yaml_bw::Value>,
}

/// Parse an AsyncAPI specification
fn parse_asyncapi(content: &str, relative_path: &str) -> Result<Vec<EndpointInfo>, String> {
    let mut endpoints = Vec::new();

    // Try JSON first, then YAML
    let spec: AsyncApiSpec = if content.trim().starts_with('{') {
        serde_json::from_str(content).map_err(|e| format!("JSON parse error: {}", e))?
    } else {
        serde_yaml_bw::from_str(content).map_err(|e| format!("YAML parse error: {}", e))?
    };

    // Get service name from title or file stem
    let service_name = spec
        .info
        .as_ref()
        .and_then(|i| i.title.as_ref())
        .cloned()
        .unwrap_or_else(|| {
            std::path::Path::new(relative_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("asyncapi")
                .to_string()
        });

    // Extract endpoints from channels
    if let Some(channels) = spec.channels {
        for (channel_name, channel) in channels.iter() {
            // Process publish operation
            if let Some(publish_op) = &channel.publish {
                endpoints.push(EndpointInfo {
                    service_name: service_name.clone(),
                    method: "publish".to_string(),
                    path: channel_name.clone(),
                    handler: publish_op.operation_id.clone(),
                    kind: "asyncapi".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "spec:asyncapi".to_string(),
                });
            }

            // Process subscribe operation
            if let Some(subscribe_op) = &channel.subscribe {
                endpoints.push(EndpointInfo {
                    service_name: service_name.clone(),
                    method: "subscribe".to_string(),
                    path: channel_name.clone(),
                    handler: subscribe_op.operation_id.clone(),
                    kind: "asyncapi".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "spec:asyncapi".to_string(),
                });
            }
        }
    }

    Ok(endpoints)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asyncapi_yaml_channels() {
        let yaml = r#"
asyncapi: "2.6.0"
info:
  title: User Events
  version: "1.0.0"
channels:
  user/created:
    subscribe:
      operationId: onUserCreated
  user/deleted:
    publish:
      operationId: deleteUser
"#;

        let endpoints = parse_asyncapi(yaml, "asyncapi.yaml").expect("parse failed");

        assert_eq!(endpoints.len(), 2);
        assert!(endpoints
            .iter()
            .any(|e| { e.method == "subscribe" && e.path == "user/created" }));
        assert!(endpoints
            .iter()
            .any(|e| { e.method == "publish" && e.path == "user/deleted" }));

        for endpoint in &endpoints {
            assert_eq!(endpoint.kind, "asyncapi");
            assert_eq!(endpoint.extraction_method, "spec:asyncapi");
            assert_eq!(endpoint.confidence, Confidence::High);
        }
    }

    #[test]
    fn test_asyncapi_with_messages() {
        let yaml = r#"
asyncapi: "2.6.0"
info:
  title: Event Stream
  version: "1.0.0"
channels:
  orders/placed:
    subscribe:
      operationId: onOrderPlaced
      message:
        name: OrderPlacedEvent
        payload:
          type: object
          properties:
            orderId:
              type: string
"#;

        let endpoints = parse_asyncapi(yaml, "events.yaml").expect("parse failed");

        assert!(endpoints.len() > 0);
        let order_event = endpoints.iter().find(|e| e.path == "orders/placed");
        assert!(order_event.is_some());
    }

    #[test]
    fn test_asyncapi_invalid_yaml() {
        let invalid = "{{{broken yaml";
        let result = parse_asyncapi(invalid, "async.yaml");
        assert!(result.is_err(), "invalid YAML should fail");
    }

    #[test]
    fn test_plugin_file_patterns() {
        let plugin = AsyncApiPlugin;
        let patterns = plugin.file_patterns();
        assert!(patterns.contains(&"**/asyncapi.yaml"));
        assert!(patterns.contains(&"**/asyncapi.json"));
    }

    #[test]
    fn test_plugin_always_run() {
        let plugin = AsyncApiPlugin;
        assert!(plugin.always_run());
    }
}
