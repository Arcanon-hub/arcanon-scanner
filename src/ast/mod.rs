//! AST wrapper around tree-sitter for safe multi-language parsing.
//!
//! # Purpose
//! Provides AstHelper which wraps tree-sitter's Parser and QueryCursor to:
//! - Hide parser internals (callers don't touch tree-sitter directly)
//! - Execute S-expression queries safely with automatic query compilation
//! - Return structured QueryMatch with text and line numbers
//!
//! # Important Notes
//! - query_matches() creates a new QueryCursor per call and drops the parse tree before returning.
//!   This is by design (MEMORY constraint — no AST retention).
//! - For production plugins, callers should cache the compiled Query in std::sync::OnceLock
//!   and call tree-sitter directly to avoid recompiling per file.
//! - query_matches() is a convenience for tests and simple single-query cases.

use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator};

/// Wrapper around tree-sitter Parser for a specific language.
/// Provides safe query execution without exposing internal tree-sitter types.
pub struct AstHelper {
    language: Language,
}

/// A single query match from AstHelper::query_matches().
/// Hides tree-sitter node types and provides convenient text + line access.
#[derive(Debug, Clone)]
pub struct QueryMatch {
    pub capture_name: String,
    pub node_text: String,
    pub line: usize, // 1-indexed
}

impl AstHelper {
    /// Create a new AstHelper for a given tree-sitter language.
    pub fn new(language: Language) -> Self {
        Self { language }
    }

    /// Execute a tree-sitter S-expression query against source code.
    ///
    /// # Behavior
    /// - Compiles the query once per call (for simplicity; production code should cache)
    /// - Creates a new Parser and parses the source
    /// - Executes the query with a fresh QueryCursor (never reused across files)
    /// - Extracts all captures with name, text (quotes trimmed), and 1-indexed line number
    /// - Drops the parse tree before returning (no AST retention — MEMORY constraint)
    ///
    /// # Returns
    /// Vec<QueryMatch> with all matches. Empty if query is invalid or parse fails.
    ///
    /// # Error Handling
    /// - Invalid query: logs error, returns empty Vec
    /// - Parse failure: returns empty Vec
    pub fn query_matches(&self, source: &str, query_src: &str) -> Vec<QueryMatch> {
        // Compile query — for production, use std::sync::OnceLock and call tree_sitter directly
        let query = match Query::new(&self.language, query_src) {
            Ok(q) => q,
            Err(e) => {
                tracing::error!("Invalid tree-sitter query: {e}");
                return vec![];
            }
        };

        let mut parser = Parser::new();
        if parser.set_language(&self.language).is_err() {
            tracing::error!("Failed to set parser language");
            return vec![];
        }

        let tree = match parser.parse(source.as_bytes(), None) {
            Some(t) => t,
            None => {
                tracing::debug!("Failed to parse source (grammar error or too large)");
                return vec![];
            }
        };

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

        let capture_names = query.capture_names();
        let mut results = Vec::new();

        // StreamingIterator::next() returns Option<&'a QueryMatch>
        while let Some(m) = matches.next() {
            for capture in m.captures {
                let name = capture_names[capture.index as usize].to_string();
                let text = capture
                    .node
                    .utf8_text(source.as_bytes())
                    .unwrap_or("")
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                let line = capture.node.start_position().row + 1; // 1-indexed
                results.push(QueryMatch {
                    capture_name: name,
                    node_text: text,
                    line,
                });
            }
        }

        // tree is dropped here — no AST retention per MEMORY constraint
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_matches_string_literal() {
        let lang: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let helper = AstHelper::new(lang);

        let source = r#"const x = "hello";"#;
        let query = r#"(string) @string"#;

        let matches = helper.query_matches(source, query);

        assert!(!matches.is_empty());
        let m = &matches[0];
        assert_eq!(m.capture_name, "string");
        assert_eq!(m.node_text, "hello"); // quotes trimmed
        assert_eq!(m.line, 1); // 1-indexed
    }

    #[test]
    fn test_query_matches_invalid_query() {
        let lang: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let helper = AstHelper::new(lang);

        let source = "const x = 1;";
        let query = "(@@@invalid@@@)"; // invalid S-expression

        let matches = helper.query_matches(source, query);
        assert!(matches.is_empty()); // should return empty, not panic
    }

    #[test]
    fn test_query_matches_no_matches() {
        let lang: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let helper = AstHelper::new(lang);

        let source = "const x = 1;";
        let query = r#"(string) @string"#; // no string literals in source

        let matches = helper.query_matches(source, query);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_query_matches_single_quotes() {
        let lang: Language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let helper = AstHelper::new(lang);

        let source = "const x = 'world';";
        let query = r#"(string) @string"#;

        let matches = helper.query_matches(source, query);
        assert!(!matches.is_empty());
        let m = &matches[0];
        assert_eq!(m.node_text, "world"); // single quotes also trimmed
    }
}
