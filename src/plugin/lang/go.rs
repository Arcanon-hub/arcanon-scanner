// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;

/// Go language plugin.
/// Covers .go and go.mod.
pub struct GoPlugin;

impl LanguagePlugin for GoPlugin {
    fn name(&self) -> &str {
        "go"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.go", "**/go.mod"]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default() // stub — implemented in Plan 04
    }
}
