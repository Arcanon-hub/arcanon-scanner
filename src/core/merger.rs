//! Service deduplication and aggregation from multiple plugin results.
//! Deduplicates services by root_path proximity, applies name priority rules,
//! and aggregates endpoints, connections, and schemas with spec-file priority.

use crate::types::*;
use std::collections::{HashMap, HashSet};

/// Result of merging multiple ExtractionResults into a single consolidated view.
#[derive(Debug, Clone)]
pub struct MergedResult {
    pub services: Vec<ServiceInfo>,
    pub endpoints: Vec<EndpointInfo>,
    pub connections: Vec<ConnectionInfo>,
    pub schemas: Vec<SchemaInfo>,
}

/// Override configuration for a service (from .arcanon.toml [services] section).
#[derive(Debug, Clone)]
pub struct ServiceOverride {
    pub name: Option<String>,
    pub ignore: Option<bool>,
}

/// Normalize a service name: lowercase, replace underscores and spaces with hyphens.
fn normalize_name(name: &str) -> String {
    name.to_lowercase().replace(['_', ' '], "-")
}

/// Priority scoring for extraction method in service name selection.
/// Higher priority wins when merging duplicate services by root_path.
fn service_priority(extraction_method: &str) -> u8 {
    match extraction_method {
        "compose" => 3,
        "package_json" => 2,
        "dockerfile" => 1,
        _ => 0,
    }
}

/// Merge multiple ExtractionResults from different plugins.
///
/// Steps:
/// 1. Deduplicate services by root_path (or normalized name if root_path is empty)
/// 2. Apply name priority: compose > package_json > dockerfile > inferred
/// 3. Aggregate all endpoints, with spec-file endpoints overriding ast-source endpoints
/// 4. Aggregate all connections (no dedup at scan level)
/// 5. Aggregate schemas, with spec-file schemas overriding ast-source schemas
pub fn merge(results: Vec<ExtractionResult>) -> MergedResult {
    // Step 1: Deduplicate services by root_path
    let mut by_key: HashMap<String, ServiceInfo> = HashMap::new();

    for result in &results {
        for svc in &result.services {
            let key = if svc.root_path.is_empty() {
                normalize_name(&svc.name)
            } else {
                svc.root_path.clone()
            };

            by_key
                .entry(key)
                .and_modify(|existing| {
                    // If new service has higher priority extraction method, use its name
                    if service_priority(&svc.extraction_method)
                        > service_priority(&existing.extraction_method)
                    {
                        existing.name = svc.name.clone();
                        existing.extraction_method = svc.extraction_method.clone();
                    }
                    // Fill in missing language if new one is available
                    if existing.language.is_empty() && !svc.language.is_empty() {
                        existing.language = svc.language.clone();
                    }
                    // Fill in boundary_entry if not already set
                    if existing.boundary_entry.is_none() {
                        existing.boundary_entry = svc.boundary_entry.clone();
                    }
                })
                .or_insert_with(|| svc.clone());
        }
    }

    let services: Vec<ServiceInfo> = by_key.into_values().collect();

    // Step 2: Aggregate endpoints with spec-override
    let mut all_endpoints: Vec<EndpointInfo> = results
        .iter()
        .flat_map(|r| r.endpoints.iter().cloned())
        .collect();

    // Spec-file endpoints override ast-source endpoints for same (service_name, method, path)
    let (spec_eps, ast_eps): (Vec<_>, Vec<_>) = all_endpoints
        .drain(..)
        .partition(|ep| ep.extraction_method.starts_with("spec:"));

    let spec_keys: HashSet<(String, String, String)> = spec_eps
        .iter()
        .map(|ep| (ep.service_name.clone(), ep.method.clone(), ep.path.clone()))
        .collect();

    let deduped_ast_eps: Vec<EndpointInfo> = ast_eps
        .into_iter()
        .filter(|ep| {
            !spec_keys.contains(&(ep.service_name.clone(), ep.method.clone(), ep.path.clone()))
        })
        .collect();

    let endpoints: Vec<EndpointInfo> = spec_eps.into_iter().chain(deduped_ast_eps).collect();

    // Step 3: Aggregate connections (no dedup at scan level; hub deduplicates cross-scan)
    let connections: Vec<ConnectionInfo> = results
        .iter()
        .flat_map(|r| r.connections.iter().cloned())
        .collect();

    // Step 4: Aggregate schemas with spec-override
    let all_schemas: Vec<SchemaInfo> = results
        .iter()
        .flat_map(|r| r.schemas.iter().cloned())
        .collect();

    // Spec-file schemas override ast-source schemas with same name
    let (spec_schemas, ast_schemas): (Vec<_>, Vec<_>) = all_schemas
        .into_iter()
        .partition(|s| s.extraction_method.starts_with("spec:"));

    let spec_schema_names: HashSet<String> = spec_schemas.iter().map(|s| s.name.clone()).collect();

    let deduped_ast_schemas: Vec<SchemaInfo> = ast_schemas
        .into_iter()
        .filter(|s| !spec_schema_names.contains(&s.name))
        .collect();

    let schemas: Vec<SchemaInfo> = spec_schemas
        .into_iter()
        .chain(deduped_ast_schemas)
        .collect();

    MergedResult {
        services,
        endpoints,
        connections,
        schemas,
    }
}

/// Check if merged result has no services and emit a warning.
/// Empty services are valid (e.g., infrastructure-only repos), but it's useful to log this.
/// Called before upload to notify the operator of potentially unexpected scan results.
pub fn check_empty_findings(merged: &MergedResult) {
    if merged.services.is_empty() {
        tracing::warn!(
            "No services detected. Add a Dockerfile, docker-compose.yml, or configure [services] in .arcanon.toml. \
             Connections ({}) and schemas ({}) will still be uploaded.",
            merged.connections.len(),
            merged.schemas.len()
        );
    }
}

/// Apply service name overrides and ignore rules from .arcanon.toml [services] section.
/// Removes ignored services and their associated endpoints.
pub fn apply_service_overrides(
    merged: &mut MergedResult,
    overrides: &HashMap<String, ServiceOverride>,
) {
    // Apply name overrides and mark ignored services
    for svc in &mut merged.services {
        if let Some(ov) = overrides.get(&svc.root_path) {
            if let Some(name) = &ov.name {
                svc.name = name.clone();
            }
            if ov.ignore.unwrap_or(false) {
                svc.service_type = "ignored".to_string();
            }
        }
    }

    // Collect names of ignored services
    let ignored: HashSet<String> = merged
        .services
        .iter()
        .filter(|s| s.service_type == "ignored")
        .map(|s| s.name.clone())
        .collect();

    // Remove ignored services and their endpoints
    merged.services.retain(|s| s.service_type != "ignored");
    merged
        .endpoints
        .retain(|ep| !ignored.contains(&ep.service_name));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_name() {
        assert_eq!(normalize_name("my_service"), "my-service");
        assert_eq!(normalize_name("My Service"), "my-service");
        assert_eq!(normalize_name("UPPERCASE"), "uppercase");
        assert_eq!(normalize_name("already-hyphenated"), "already-hyphenated");
    }

    #[test]
    fn test_service_priority() {
        assert_eq!(service_priority("compose"), 3);
        assert_eq!(service_priority("package_json"), 2);
        assert_eq!(service_priority("dockerfile"), 1);
        assert_eq!(service_priority("ast:typescript"), 0);
        assert_eq!(service_priority("unknown"), 0);
    }

    #[test]
    fn test_merge_services_same_root_path_different_names() {
        // Two services with same root_path but different names
        // Compose version should win (higher priority)
        let result1 = ExtractionResult {
            services: vec![ServiceInfo {
                name: "api_server".to_string(),
                root_path: "services/api".to_string(),
                language: "typescript".to_string(),
                service_type: "service".to_string(),
                boundary_entry: Some("src/main.ts".to_string()),
                confidence: Confidence::High,
                extraction_method: "dockerfile".to_string(),
            }],
            endpoints: vec![],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let result2 = ExtractionResult {
            services: vec![ServiceInfo {
                name: "api".to_string(),
                root_path: "services/api".to_string(),
                language: "".to_string(),
                service_type: "service".to_string(),
                boundary_entry: None,
                confidence: Confidence::High,
                extraction_method: "compose".to_string(),
            }],
            endpoints: vec![],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let merged = merge(vec![result1, result2]);
        assert_eq!(merged.services.len(), 1);
        assert_eq!(merged.services[0].name, "api"); // compose wins
        assert_eq!(merged.services[0].root_path, "services/api");
        assert_eq!(merged.services[0].extraction_method, "compose");
    }

    #[test]
    fn test_merge_services_different_root_paths() {
        // Two services with different root_paths should remain separate
        let result = ExtractionResult {
            services: vec![
                ServiceInfo {
                    name: "api".to_string(),
                    root_path: "services/api".to_string(),
                    language: "typescript".to_string(),
                    service_type: "service".to_string(),
                    boundary_entry: None,
                    confidence: Confidence::High,
                    extraction_method: "compose".to_string(),
                },
                ServiceInfo {
                    name: "worker".to_string(),
                    root_path: "services/worker".to_string(),
                    language: "python".to_string(),
                    service_type: "service".to_string(),
                    boundary_entry: None,
                    confidence: Confidence::High,
                    extraction_method: "compose".to_string(),
                },
            ],
            endpoints: vec![],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let merged = merge(vec![result]);
        assert_eq!(merged.services.len(), 2);
        assert!(merged.services.iter().any(|s| s.name == "api"));
        assert!(merged.services.iter().any(|s| s.name == "worker"));
    }

    #[test]
    fn test_merge_endpoints_spec_override() {
        // Spec endpoint and ast endpoint with same (service, method, path)
        // Spec should win, ast should be dropped
        let result1 = ExtractionResult {
            services: vec![],
            endpoints: vec![EndpointInfo {
                service_name: "api".to_string(),
                method: "POST".to_string(),
                path: "/api/v1/orders".to_string(),
                handler: Some("createOrder".to_string()),
                kind: "rest".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
            }],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let result2 = ExtractionResult {
            services: vec![],
            endpoints: vec![EndpointInfo {
                service_name: "api".to_string(),
                method: "POST".to_string(),
                path: "/api/v1/orders".to_string(),
                handler: Some("orders".to_string()),
                kind: "rest".to_string(),
                confidence: Confidence::High,
                extraction_method: "spec:openapi".to_string(),
            }],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let merged = merge(vec![result1, result2]);
        assert_eq!(merged.endpoints.len(), 1);
        assert_eq!(merged.endpoints[0].extraction_method, "spec:openapi");
        assert_eq!(merged.endpoints[0].handler, Some("orders".to_string()));
    }

    #[test]
    fn test_merge_connections_aggregated() {
        // Connections from multiple plugins are aggregated (no dedup)
        let result1 = ExtractionResult {
            services: vec![],
            endpoints: vec![],
            connections: vec![ConnectionInfo {
                source_service: "api".to_string(),
                target_name: "payment-service".to_string(),
                protocol: "rest".to_string(),
                method: Some("POST".to_string()),
                path: Some("/api/v1/payments".to_string()),
                source_file: "src/services/payment-client.ts:42".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
                evidence: Some("axios.post(...)".to_string()),
            }],
            schemas: vec![],
            actors: vec![],
        };

        let result2 = ExtractionResult {
            services: vec![],
            endpoints: vec![],
            connections: vec![ConnectionInfo {
                source_service: "api".to_string(),
                target_name: "database".to_string(),
                protocol: "postgresql".to_string(),
                method: None,
                path: None,
                source_file: "src/models/user.ts:15".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
                evidence: None,
            }],
            schemas: vec![],
            actors: vec![],
        };

        let merged = merge(vec![result1, result2]);
        assert_eq!(merged.connections.len(), 2);
    }

    #[test]
    fn test_merge_schemas_spec_override() {
        // Spec schema and ast schema with same name
        // Spec should win, ast should be dropped
        let result1 = ExtractionResult {
            services: vec![],
            endpoints: vec![],
            connections: vec![],
            schemas: vec![SchemaInfo {
                name: "CreateOrderRequest".to_string(),
                role: "request".to_string(),
                file: Some("src/models/order.ts".to_string()),
                connection_ref: None,
                fields: vec![FieldInfo {
                    name: "product_id".to_string(),
                    field_type: "string".to_string(),
                    required: true,
                }],
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
            }],
            actors: vec![],
        };

        let result2 = ExtractionResult {
            services: vec![],
            endpoints: vec![],
            connections: vec![],
            schemas: vec![SchemaInfo {
                name: "CreateOrderRequest".to_string(),
                role: "request".to_string(),
                file: Some("openapi.yaml".to_string()),
                connection_ref: None,
                fields: vec![
                    FieldInfo {
                        name: "productId".to_string(),
                        field_type: "string".to_string(),
                        required: true,
                    },
                    FieldInfo {
                        name: "quantity".to_string(),
                        field_type: "integer".to_string(),
                        required: false,
                    },
                ],
                confidence: Confidence::High,
                extraction_method: "spec:openapi".to_string(),
            }],
            actors: vec![],
        };

        let merged = merge(vec![result1, result2]);
        assert_eq!(merged.schemas.len(), 1);
        assert_eq!(merged.schemas[0].extraction_method, "spec:openapi");
        assert_eq!(merged.schemas[0].fields.len(), 2);
    }

    #[test]
    fn test_apply_service_overrides_name_change() {
        let mut merged = MergedResult {
            services: vec![ServiceInfo {
                name: "api_service".to_string(),
                root_path: "services/api".to_string(),
                language: "typescript".to_string(),
                service_type: "service".to_string(),
                boundary_entry: None,
                confidence: Confidence::High,
                extraction_method: "compose".to_string(),
            }],
            endpoints: vec![],
            connections: vec![],
            schemas: vec![],
        };

        let mut overrides = HashMap::new();
        overrides.insert(
            "services/api".to_string(),
            ServiceOverride {
                name: Some("order-processor".to_string()),
                ignore: None,
            },
        );

        apply_service_overrides(&mut merged, &overrides);
        assert_eq!(merged.services[0].name, "order-processor");
    }

    #[test]
    fn test_apply_service_overrides_ignore() {
        let mut merged = MergedResult {
            services: vec![
                ServiceInfo {
                    name: "api".to_string(),
                    root_path: "services/api".to_string(),
                    language: "typescript".to_string(),
                    service_type: "service".to_string(),
                    boundary_entry: None,
                    confidence: Confidence::High,
                    extraction_method: "compose".to_string(),
                },
                ServiceInfo {
                    name: "temp-worker".to_string(),
                    root_path: "services/temp".to_string(),
                    language: "python".to_string(),
                    service_type: "service".to_string(),
                    boundary_entry: None,
                    confidence: Confidence::High,
                    extraction_method: "compose".to_string(),
                },
            ],
            endpoints: vec![
                EndpointInfo {
                    service_name: "api".to_string(),
                    method: "GET".to_string(),
                    path: "/health".to_string(),
                    handler: None,
                    kind: "rest".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "ast:typescript".to_string(),
                },
                EndpointInfo {
                    service_name: "temp-worker".to_string(),
                    method: "GET".to_string(),
                    path: "/status".to_string(),
                    handler: None,
                    kind: "rest".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "ast:python".to_string(),
                },
            ],
            connections: vec![],
            schemas: vec![],
        };

        let mut overrides = HashMap::new();
        overrides.insert(
            "services/temp".to_string(),
            ServiceOverride {
                name: None,
                ignore: Some(true),
            },
        );

        apply_service_overrides(&mut merged, &overrides);
        assert_eq!(merged.services.len(), 1);
        assert_eq!(merged.services[0].name, "api");
        assert_eq!(merged.endpoints.len(), 1);
        assert_eq!(merged.endpoints[0].service_name, "api");
    }
}
