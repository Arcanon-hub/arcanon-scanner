pub mod openapi;
pub use openapi::OpenApiPlugin;

pub mod dockerfile;
pub use dockerfile::DockerfilePlugin;

pub mod env;
pub use env::EnvPlugin;

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;

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
