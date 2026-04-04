use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;

/// OpenAPI/Swagger specification parser (always runs).
pub struct OpenApiPlugin;

impl LanguagePlugin for OpenApiPlugin {
    fn name(&self) -> &str {
        "openapi"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// Protocol Buffers (.proto) file parser (always runs).
pub struct ProtoPlugin;

impl LanguagePlugin for ProtoPlugin {
    fn name(&self) -> &str {
        "proto"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// GraphQL schema parser (always runs).
pub struct GraphqlPlugin;

impl LanguagePlugin for GraphqlPlugin {
    fn name(&self) -> &str {
        "graphql"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// AsyncAPI specification parser (always runs).
pub struct AsyncApiPlugin;

impl LanguagePlugin for AsyncApiPlugin {
    fn name(&self) -> &str {
        "asyncapi"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// Docker Compose manifest parser (always runs).
pub struct ComposePlugin;

impl LanguagePlugin for ComposePlugin {
    fn name(&self) -> &str {
        "compose"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// Kubernetes manifest parser (always runs).
pub struct KubernetesPlugin;

impl LanguagePlugin for KubernetesPlugin {
    fn name(&self) -> &str {
        "kubernetes"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// Dockerfile/Containerfile parser (always runs).
pub struct DockerfilePlugin;

impl LanguagePlugin for DockerfilePlugin {
    fn name(&self) -> &str {
        "dockerfile"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// .env environment file parser (always runs).
pub struct EnvPlugin;

impl LanguagePlugin for EnvPlugin {
    fn name(&self) -> &str {
        "env"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}
