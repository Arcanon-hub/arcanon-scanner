use anyhow::Result;
use tree_sitter::{Language, Parser};

/// Wrapper around tree-sitter Parser for a specific language.
#[allow(dead_code)]
pub struct AstParser {
    parser: Parser,
}

impl AstParser {
    #[allow(dead_code)]
    pub fn new(language: Language) -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&language)
            .map_err(|e| anyhow::anyhow!("tree-sitter language init failed: {}", e))?;
        Ok(Self { parser })
    }

    #[allow(dead_code)]
    pub fn parse(&mut self, source: &str) -> Option<tree_sitter::Tree> {
        self.parser.parse(source, None)
    }
}
