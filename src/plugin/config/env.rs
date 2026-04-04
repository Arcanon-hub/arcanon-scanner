use crate::plugin::{ExtractionContext, LanguagePlugin};
use crate::types::ExtractionResult;

/// Environment file parser — detects .env files (actual parsing is in vars/mod.rs).
/// This plugin acts as a marker; the ExtractionResult is always empty.
pub struct EnvPlugin;

impl LanguagePlugin for EnvPlugin {
    fn name(&self) -> &str {
        "env"
    }

    fn file_patterns(&self) -> &[&str] {
        &[
            "**/.env",
            "**/.env.local",
            "**/.env.development",
            "**/.env.production",
            "**/.env.*",
        ]
    }

    fn always_run(&self) -> bool {
        true
    }

    fn extract(&self, _ctx: &ExtractionContext) -> ExtractionResult {
        // EnvPlugin is a marker plugin. Actual env var parsing happens in
        // vars/mod.rs before plugins run. This plugin's job is just to signal
        // which .env files exist. The ExtractionResult is always empty.
        ExtractionResult::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::plugin::FileContext;
    use crate::vars::VariableStore;

    #[test]
    fn test_file_patterns() {
        let plugin = EnvPlugin;
        let patterns = plugin.file_patterns();
        assert!(patterns.contains(&"**/.env"));
        assert!(patterns.contains(&"**/.env.local"));
        assert!(patterns.contains(&"**/.env.development"));
        assert!(patterns.contains(&"**/.env.production"));
        assert!(patterns.contains(&"**/.env.*"));
    }

    #[test]
    fn test_extract_returns_empty() {
        let plugin = EnvPlugin;
        let root = PathBuf::from("/repo");
        let file = FileContext {
            path: PathBuf::from("/repo/.env"),
            relative_path: ".env".to_string(),
            content: Arc::from("DB_HOST=localhost\nDB_PORT=5432"),
        };

        let ctx = ExtractionContext {
            files: vec![file],
            vars: Arc::new(VariableStore::new()),
            root,
        };

        let result = plugin.extract(&ctx);
        assert_eq!(result.services.len(), 0);
        assert_eq!(result.endpoints.len(), 0);
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.schemas.len(), 0);
    }

    #[test]
    fn test_always_run() {
        let plugin = EnvPlugin;
        assert!(plugin.always_run());
    }

    #[test]
    fn test_name() {
        let plugin = EnvPlugin;
        assert_eq!(plugin.name(), "env");
    }
}
