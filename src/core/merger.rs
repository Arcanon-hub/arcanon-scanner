use std::collections::HashMap;

use crate::types::{ConnectionInfo, EndpointInfo, ExtractionResult, SchemaInfo, ServiceInfo};

use tracing::debug;

use serde::Deserialize;

/// Override for a specific service detected by a plugin.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ServiceOverride {
    /// Override the name of the service
    pub name: Option<String>,
    /// Completely ignore this service and its children findings
    pub ignore: Option<bool>,
}

/// The final merged result of all extractions.
#[derive(Debug, Clone, Default)]
pub struct MergedResult {
    pub services: Vec<ServiceInfo>,
    pub endpoints: Vec<EndpointInfo>,
    pub connections: Vec<ConnectionInfo>,
    pub schemas: Vec<SchemaInfo>,
    pub actors: Vec<crate::types::ActorInfo>,
}

/// Merge multiple extraction results into a single result.
/// - Services are merged by root_path.
/// - Later results in the input vector overwrite earlier ones for the same root_path.
/// - Endpoints, connections, and schemas are collected from all results.
pub fn merge(results: Vec<ExtractionResult>) -> MergedResult {
    let mut merged = MergedResult::default();
    let mut service_map: HashMap<String, ServiceInfo> = HashMap::new();

    for res in results {
        for service in res.services {
            service_map
                .entry(service.root_path.clone())
                .and_modify(|existing| {
                    // compose always beats dockerfile; any other source keeps the first winner
                    if existing.extraction_method == "dockerfile"
                        && service.extraction_method == "compose"
                    {
                        *existing = service.clone();
                    }
                })
                .or_insert(service);
        }
        merged.endpoints.extend(res.endpoints);
        merged.connections.extend(res.connections);
        merged.schemas.extend(res.schemas);
        merged.actors.extend(res.actors);
    }

    merged.services = service_map.into_values().collect();

    // Deduplicate endpoints: spec:openapi/spec:asyncapi entries suppress duplicates
    // for the same (service_name, method, path) key.
    {
        use std::collections::HashSet;
        let spec_keys: HashSet<(String, String, String)> = merged
            .endpoints
            .iter()
            .filter(|e| {
                e.extraction_method == "spec:openapi" || e.extraction_method == "spec:asyncapi"
            })
            .map(|e| (e.service_name.clone(), e.method.clone(), e.path.clone()))
            .collect();
        if !spec_keys.is_empty() {
            merged.endpoints.retain(|e| {
                e.extraction_method == "spec:openapi"
                    || e.extraction_method == "spec:asyncapi"
                    || !spec_keys.contains(&(
                        e.service_name.clone(),
                        e.method.clone(),
                        e.path.clone(),
                    ))
            });
        }
    }

    // Deduplicate schemas: spec entries suppress AST duplicates with the same name.
    {
        use std::collections::HashSet;
        let spec_schema_names: HashSet<String> = merged
            .schemas
            .iter()
            .filter(|s| {
                s.extraction_method == "spec:openapi" || s.extraction_method == "spec:asyncapi"
            })
            .map(|s| s.name.clone())
            .collect();
        if !spec_schema_names.is_empty() {
            merged.schemas.retain(|s| {
                s.extraction_method == "spec:openapi"
                    || s.extraction_method == "spec:asyncapi"
                    || !spec_schema_names.contains(&s.name)
            });
        }
    }

    merged
}

/// If no services were detected (or all were ignored), and we have a repo name,
/// infer a single service at the root to avoid empty results.
pub fn infer_service_if_needed(merged: &mut MergedResult, repo_name: &str) {
    if merged.services.is_empty()
        && (!merged.connections.is_empty()
            || !merged.schemas.is_empty()
            || !merged.endpoints.is_empty())
    {
        debug!("No services detected, inferring service '{}' at root", repo_name);
        merged.services.push(crate::types::ServiceInfo {
            name: repo_name.to_string(),
            root_path: ".".to_string(),
            language: String::new(),
            service_type: "service".to_string(),
            boundary_entry: None,
            confidence: crate::types::Confidence::High,
            extraction_method: "inferred_root".to_string(),
            ..Default::default()
        });
    }
}

/// Apply user-defined service name/ignore overrides to the merged result.
pub fn apply_service_overrides(merged: &mut MergedResult, overrides: &HashMap<String, ServiceOverride>) {
    // 1. Filter out ignored services
    let ignored_roots: Vec<String> = overrides
        .iter()
        .filter(|(_, v)| v.ignore.unwrap_or(false))
        .map(|(k, _)| k.clone())
        .collect();

    if !ignored_roots.is_empty() {
        debug!("Applying ignore overrides for: {:?}", ignored_roots);
        merged.services.retain(|s| !ignored_roots.contains(&s.root_path));
        merged.endpoints.retain(|e| !ignored_roots.iter().any(|root| e.service_name == *root));
        merged.connections.retain(|c| !ignored_roots.iter().any(|root| c.source_service == *root));
    }

    // 2. Apply name overrides
    for (root, ovr) in overrides {
        if let Some(new_name) = &ovr.name {
            debug!("Overriding service name at {}: {}", root, new_name);
            for s in &mut merged.services {
                if s.root_path == *root {
                    s.name = new_name.clone();
                }
            }
            // Update names in endpoints/connections too
            for e in &mut merged.endpoints {
                if e.service_name == *root {
                    e.service_name = new_name.clone();
                }
            }
            for c in &mut merged.connections {
                if c.source_service == *root {
                    c.source_service = new_name.clone();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Confidence;

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
                ..Default::default()
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
                ..Default::default()
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
                    extraction_method: "test".to_string(),
                    ..Default::default()
                },
                ServiceInfo {
                    name: "worker".to_string(),
                    root_path: "services/worker".to_string(),
                    language: "python".to_string(),
                    service_type: "service".to_string(),
                    boundary_entry: None,
                    confidence: Confidence::High,
                    extraction_method: "test".to_string(),
                    ..Default::default()
                }
            ],
            endpoints: vec![],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let merged = merge(vec![result]);
        assert_eq!(merged.services.len(), 2);
    }

    #[test]
    fn test_apply_service_overrides_name() {
        let mut merged = MergedResult {
            services: vec![ServiceInfo {
                name: "old-name".to_string(),
                root_path: "services/api".to_string(),
                language: "typescript".to_string(),
                service_type: "service".to_string(),
                boundary_entry: None,
                confidence: Confidence::High,
                extraction_method: "compose".to_string(),
                ..Default::default()
            }],
            endpoints: vec![EndpointInfo {
                service_name: "services/api".to_string(),
                method: "GET".to_string(),
                path: "/".to_string(),
                handler: None,
                kind: "rest".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
                ..Default::default()
            }],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
        };

        let mut overrides = HashMap::new();
        overrides.insert(
            "services/api".to_string(),
            ServiceOverride {
                name: Some("new-name".to_string()),
                ignore: None,
            },
        );

        apply_service_overrides(&mut merged, &overrides);
        assert_eq!(merged.services[0].name, "new-name");
        assert_eq!(merged.endpoints[0].service_name, "new-name");
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
                    extraction_method: "test".to_string(),
                    ..Default::default()
                },
                ServiceInfo {
                    name: "temp-worker".to_string(),
                    root_path: "services/temp".to_string(),
                    language: "python".to_string(),
                    service_type: "service".to_string(),
                    boundary_entry: None,
                    confidence: Confidence::High,
                    extraction_method: "test".to_string(),
                    ..Default::default()
                }
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
                    ..Default::default()
                },
                EndpointInfo {
                    service_name: "services/temp".to_string(),
                    method: "GET".to_string(),
                    path: "/status".to_string(),
                    handler: None,
                    kind: "rest".to_string(),
                    confidence: Confidence::High,
                    extraction_method: "ast:python".to_string(),
                    ..Default::default()
                },
            ],
            connections: vec![],
            schemas: vec![],
            actors: vec![],
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
