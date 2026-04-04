// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;

/// TypeScript/JavaScript language plugin.
/// Covers .ts, .tsx, .js, .jsx, and package.json for framework detection.
pub struct TypeScriptPlugin;

impl LanguagePlugin for TypeScriptPlugin {
    fn name(&self) -> &str {
        "typescript"
    }

    fn file_patterns(&self) -> &[&str] {
        &[
            "**/*.ts",
            "**/*.tsx",
            "**/*.js",
            "**/*.jsx",
            "**/package.json", // needed for framework marker detection (LPLU-08)
        ]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default() // stub — implemented in Plan 02
    }
}
