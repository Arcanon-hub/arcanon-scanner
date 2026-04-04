use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;

/// TypeScript/JavaScript language plugin.
pub struct TypeScriptPlugin;

impl LanguagePlugin for TypeScriptPlugin {
    fn name(&self) -> &str {
        "typescript"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// Python language plugin.
pub struct PythonPlugin;

impl LanguagePlugin for PythonPlugin {
    fn name(&self) -> &str {
        "python"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// Go language plugin.
pub struct GoPlugin;

impl LanguagePlugin for GoPlugin {
    fn name(&self) -> &str {
        "go"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// Java language plugin.
pub struct JavaPlugin;

impl LanguagePlugin for JavaPlugin {
    fn name(&self) -> &str {
        "java"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// C# language plugin.
pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn name(&self) -> &str {
        "csharp"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// Rust language plugin.
pub struct RustLangPlugin;

impl LanguagePlugin for RustLangPlugin {
    fn name(&self) -> &str {
        "rust"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}

/// Ruby language plugin.
pub struct RubyPlugin;

impl LanguagePlugin for RubyPlugin {
    fn name(&self) -> &str {
        "ruby"
    }

    fn file_patterns(&self) -> &[&str] {
        &[]
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        ExtractionResult::default()
    }
}
