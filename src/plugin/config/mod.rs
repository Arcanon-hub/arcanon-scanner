pub mod openapi;
pub use openapi::OpenApiPlugin;

pub mod proto;
pub use proto::ProtoPlugin;

pub mod dockerfile;
pub use dockerfile::DockerfilePlugin;

pub mod env;
pub use env::EnvPlugin;

pub mod compose;
pub use compose::ComposePlugin;

pub mod kubernetes;
pub use kubernetes::KubernetesPlugin;

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;

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
