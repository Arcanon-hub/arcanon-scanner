// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;

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

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default() // stub — implemented in Plan 05
    }
}
