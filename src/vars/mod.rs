use std::collections::HashMap;

/// Variable resolution store (.env, compose env, k8s ConfigMap values).
/// Full implementation is Phase 2. Phase 1 provides the stub so ExtractionContext compiles.
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct VariableStore {
    values: HashMap<String, String>,
}

impl VariableStore {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a variable name to its value. Returns None in Phase 1 stub.
    /// Phase 2 implements the full .env → compose → k8s resolution chain.
    #[allow(dead_code)]
    pub fn resolve(&self, _name: &str) -> Option<&str> {
        None
    }
}
