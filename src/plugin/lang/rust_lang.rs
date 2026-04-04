// HARD BOUNDARY: No tokio imports allowed in language plugin code.
// Plugins are synchronous (rayon). Upload is async (tokio). See PITFALLS.md Pitfall 4.

use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;

/// Rust language plugin.
/// Covers .rs and Cargo.toml.
/// Note: Module named rust_lang to avoid keyword conflict.
pub struct RustLangPlugin;

impl LanguagePlugin for RustLangPlugin {
    fn name(&self) -> &str {
        "rust"
    }

    fn file_patterns(&self) -> &[&str] {
        &["**/*.rs", "**/Cargo.toml"]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default() // stub — implemented in Plan 07
    }
}
