//! Intra-repository connection resolution.
//! Normalizes endpoint paths to match outbound connection calls and resolves target services locally.

use crate::core::merger::MergedResult;
use std::collections::HashMap;

/// Normalize a path by applying URL path parameter normalization rules.
pub fn normalize_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with(':') || (segment.starts_with('{') && segment.ends_with('}')) {
                "{param}".to_string()
            } else if segment == "*" {
                "{*}".to_string()
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Resolve intra-repository connections by matching outbound calls to local endpoints.
pub fn resolve(mut merged: MergedResult) -> MergedResult {
    let endpoint_lookup: HashMap<(String, String), String> = merged
        .endpoints
        .iter()
        .map(|ep| {
            (
                (ep.method.to_uppercase(), normalize_path(&ep.path)),
                ep.service_name.clone(),
            )
        })
        .collect();

    for conn in &mut merged.connections {
        if let (Some(method), Some(path)) = (&conn.method, &conn.path) {
            let key = (method.to_uppercase(), normalize_path(path));
            if let Some(target_svc) = endpoint_lookup.get(&key) {
                conn.target_name = target_svc.clone();
            }
        }
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    #[test]
    fn test_normalize_path_express_style() {
        assert_eq!(normalize_path("/api/v1/users/:id"), "/api/v1/users/{param}");
    }

    #[test]
    fn test_normalize_path_braces_simple() {
        assert_eq!(
            normalize_path("/api/v1/users/{userId}"),
            "/api/v1/users/{param}"
        );
    }

    #[test]
    fn test_normalize_path_braces_with_constraint() {
        assert_eq!(
            normalize_path("/api/v1/users/{id:\\d+}"),
            "/api/v1/users/{param}"
        );
    }

    #[test]
    fn test_normalize_path_wildcard() {
        assert_eq!(normalize_path("/api/v1/files/*"), "/api/v1/files/{*}");
    }

    #[test]
    fn test_normalize_path_static_unchanged() {
        assert_eq!(normalize_path("/api/v1/users"), "/api/v1/users");
    }

    #[test]
    fn test_normalize_path_complex() {
        assert_eq!(
            normalize_path("/api/v1/orders/:id/items/{itemId}/docs/*"),
            "/api/v1/orders/{param}/items/{param}/docs/{*}"
        );
    }

    #[test]
    fn test_resolve_intra_repo_connection_match() {
        let merged = MergedResult {
            services: vec![ServiceInfo {
                name: "payment-service".to_string(),
                root_path: ".".to_string(),
                language: "typescript".to_string(),
                service_type: "service".to_string(),
                boundary_entry: Some("src/main.ts".to_string()),
                confidence: Confidence::High,
                extraction_method: "compose".to_string(),
            }],
            endpoints: vec![EndpointInfo {
                service_name: "payment-service".to_string(),
                method: "POST".to_string(),
                path: "/api/v1/payments/charge".to_string(),
                handler: Some("chargePayment".to_string()),
                kind: "rest".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
            }],
            connections: vec![ConnectionInfo {
                source_service: "order-service".to_string(),
                target_name: "unknown-target".to_string(),
                protocol: "rest".to_string(),
                method: Some("POST".to_string()),
                path: Some("/api/v1/payments/charge".to_string()),
                source_file: "src/services/payment-client.ts:42".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
                evidence: Some("axios.post(...)".to_string()),
            }],
            schemas: vec![],
        };

        let resolved = resolve(merged);
        assert_eq!(resolved.connections.len(), 1);
        assert_eq!(resolved.connections[0].target_name, "payment-service");
    }

    #[test]
    fn test_resolve_no_matching_endpoint() {
        let merged = MergedResult {
            services: vec![ServiceInfo {
                name: "api".to_string(),
                root_path: ".".to_string(),
                language: "typescript".to_string(),
                service_type: "service".to_string(),
                boundary_entry: None,
                confidence: Confidence::High,
                extraction_method: "compose".to_string(),
            }],
            endpoints: vec![EndpointInfo {
                service_name: "api".to_string(),
                method: "GET".to_string(),
                path: "/health".to_string(),
                handler: None,
                kind: "rest".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
            }],
            connections: vec![ConnectionInfo {
                source_service: "api".to_string(),
                target_name: "external-payment-api".to_string(),
                protocol: "rest".to_string(),
                method: Some("POST".to_string()),
                path: Some("/api/v1/payments".to_string()),
                source_file: "src/payment.ts:10".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
                evidence: None,
            }],
            schemas: vec![],
        };

        let resolved = resolve(merged);
        assert_eq!(resolved.connections.len(), 1);
        assert_eq!(resolved.connections[0].target_name, "external-payment-api");
    }

    #[test]
    fn test_resolve_parameterized_path_match() {
        let merged = MergedResult {
            services: vec![ServiceInfo {
                name: "user-service".to_string(),
                root_path: ".".to_string(),
                language: "go".to_string(),
                service_type: "service".to_string(),
                boundary_entry: None,
                confidence: Confidence::High,
                extraction_method: "compose".to_string(),
            }],
            endpoints: vec![EndpointInfo {
                service_name: "user-service".to_string(),
                method: "GET".to_string(),
                path: "/api/v1/users/{id}".to_string(),
                handler: Some("getUser".to_string()),
                kind: "rest".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:go".to_string(),
            }],
            connections: vec![ConnectionInfo {
                source_service: "order-service".to_string(),
                target_name: "unknown".to_string(),
                protocol: "rest".to_string(),
                method: Some("GET".to_string()),
                path: Some("/api/v1/users/:userId".to_string()),
                source_file: "services/order/client.go:25".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:go".to_string(),
                evidence: None,
            }],
            schemas: vec![],
        };

        let resolved = resolve(merged);
        assert_eq!(resolved.connections[0].target_name, "user-service");
    }

    #[test]
    fn test_resolve_no_method_or_path() {
        let merged = MergedResult {
            services: vec![],
            endpoints: vec![EndpointInfo {
                service_name: "db".to_string(),
                method: "SELECT".to_string(),
                path: "/".to_string(),
                handler: None,
                kind: "database".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
            }],
            connections: vec![ConnectionInfo {
                source_service: "api".to_string(),
                target_name: "postgres".to_string(),
                protocol: "postgresql".to_string(),
                method: None,
                path: None,
                source_file: "src/db.ts:5".to_string(),
                confidence: Confidence::High,
                extraction_method: "ast:typescript".to_string(),
                evidence: None,
            }],
            schemas: vec![],
        };

        let resolved = resolve(merged);
        assert_eq!(resolved.connections[0].target_name, "postgres");
    }
}
