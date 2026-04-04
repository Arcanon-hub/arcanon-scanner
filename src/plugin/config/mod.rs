// HARD BOUNDARY: No tokio imports allowed in plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

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

pub mod graphql;
pub use graphql::GraphqlPlugin;

pub mod asyncapi;
pub use asyncapi::AsyncApiPlugin;

pub mod kubernetes;
pub use kubernetes::KubernetesPlugin;

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;
