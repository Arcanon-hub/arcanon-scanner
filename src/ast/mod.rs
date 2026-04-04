use anyhow::Result;
use tree_sitter::{Language, Parser};

/// Wrapper around tree-sitter Parser for a specific language.
pub struct AstParser {
    parser: Parser,
}

impl AstParser {
    pub fn new(language: Language) -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| anyhow::anyhow!("tree-sitter language init failed: {}", e))?;
        Ok(Self { parser })
    }

    pub fn parse(&mut self, source: &str) -> Option<tree_sitter::Tree> {
        self.parser.parse(source, None)
    }
}
