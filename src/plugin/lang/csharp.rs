// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;

/// C# language plugin.
/// Covers .cs and .csproj.
pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn name(&self) -> &str {
        "csharp"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.cs", "**/*.csproj"]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default() // stub — implemented in Plan 06
    }
}
