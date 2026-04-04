use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Service target extracted from a URL (hostname + optional port)
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceTarget {
    pub hostname: String,
    pub port: Option<u16>,
}

/// Variable resolution store with three priority layers:
/// 1. env_files (highest) — merged .env files
/// 2. compose_env (middle) — docker-compose environment entries
/// 3. k8s_env (lowest) — Kubernetes ConfigMap data
#[derive(Debug)]
#[allow(dead_code)]
pub struct VariableStore {
    env_files: HashMap<String, String>,
    compose_env: HashMap<String, String>,
    k8s_env: HashMap<String, String>,
}

impl Default for VariableStore {
    fn default() -> Self {
        Self::new()
    }
}

impl VariableStore {
    /// Create a new empty VariableStore (for testing)
    pub fn new() -> Self {
        VariableStore {
            env_files: HashMap::new(),
            compose_env: HashMap::new(),
            k8s_env: HashMap::new(),
        }
    }

    /// Resolve a variable name to its value, checking layers in priority order
    #[allow(dead_code)]
    pub fn resolve(&self, key: &str) -> Option<&str> {
        self.env_files
            .get(key)
            .or_else(|| self.compose_env.get(key))
            .or_else(|| self.k8s_env.get(key))
            .map(|s| s.as_str())
    }

    /// Resolve a variable name to a ServiceTarget by parsing it as a URL
    #[allow(dead_code)]
    pub fn resolve_to_target(&self, key: &str) -> Option<ServiceTarget> {
        let val = self.resolve(key)?;
        parse_url_to_service_target(val)
    }
}

/// Parse a URL string into a ServiceTarget (hostname + optional port)
#[allow(dead_code)]
fn parse_url_to_service_target(val: &str) -> Option<ServiceTarget> {
    let url = url::Url::parse(val).ok()?;
    let hostname = url.host_str()?.to_string();
    if hostname.is_empty() {
        return None;
    }
    let port = url.port();
    Some(ServiceTarget { hostname, port })
}

/// Build a VariableStore from files, merging three sources in priority order:
/// 1. .env files (in order: .env < .env.local < .env.development < .env.production)
/// 2. docker-compose environment (both list and map forms)
/// 3. Kubernetes ConfigMap data (from files in k8s/, kubernetes/, or manifests/ dirs)
pub fn build_variable_store(_root: &Path, files: &[PathBuf]) -> VariableStore {
    let mut env_map = HashMap::new();
    let mut compose_map = HashMap::new();
    let mut k8s_map = HashMap::new();

    // Step 1: Collect and merge .env files
    let mut env_file_paths: Vec<&PathBuf> = files
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(".env"))
        })
        .collect();

    // Sort by priority ascending (lower number = lower priority = loaded first = overwritten by later)
    env_file_paths.sort_by_key(|p| env_file_priority(p));

    for path in env_file_paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            env_map.extend(parse_env_file(&content));
        }
    }

    // Step 2: Extract docker-compose environment variables
    for path in files {
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if (filename.contains("docker-compose") || filename.contains("compose"))
            && (filename.ends_with(".yml") || filename.ends_with(".yaml"))
        {
            if let Ok(content) = std::fs::read_to_string(path) {
                compose_map.extend(extract_compose_env(&content));
            }
        }
    }

    // Step 3: Extract k8s ConfigMap data
    for path in files {
        let in_k8s_dir = path.ancestors().any(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == "k8s" || n == "kubernetes" || n == "manifests")
        });

        if in_k8s_dir {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if filename.ends_with(".yml") || filename.ends_with(".yaml") {
                if let Ok(content) = std::fs::read_to_string(path) {
                    k8s_map.extend(extract_k8s_configmap_env(&content));
                }
            }
        }
    }

    VariableStore {
        env_files: env_map,
        compose_env: compose_map,
        k8s_env: k8s_map,
    }
}

/// Determine the priority of an .env file (lower number = lower priority)
fn env_file_priority(path: &Path) -> u8 {
    match path.file_name().and_then(|n| n.to_str()).unwrap_or("") {
        ".env" => 0,
        ".env.local" => 1,
        ".env.development" => 2,
        ".env.production" => 3,
        _ => 4, // other .env.* files get lowest priority
    }
}

/// Parse a .env format file into a HashMap
fn parse_env_file(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim().to_string();
            let val = val.trim();
            let val = val
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| val.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(val)
                .to_string();
            map.insert(key, val);
        }
    }
    map
}

/// Extract environment variables from docker-compose YAML
fn extract_compose_env(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(doc) = serde_yaml_bw::from_str::<serde_yaml_bw::Value>(content) else {
        return map;
    };

    let Some(services) = doc.get("services").and_then(|s| s.as_mapping()) else {
        return map;
    };

    for (_service_name, service) in services {
        let Some(env) = service.get("environment") else {
            continue;
        };

        match env {
            // List form: ["KEY=value", "KEY2=value2"]
            serde_yaml_bw::Value::Sequence(seq) => {
                for item in seq {
                    if let Some(s) = item.as_str() {
                        if let Some((k, v)) = s.split_once('=') {
                            map.insert(k.to_string(), v.to_string());
                        }
                    }
                }
            }
            // Map form: { KEY: value }
            serde_yaml_bw::Value::Mapping(m) => {
                for (k, v) in m {
                    if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                        map.insert(k.to_string(), v.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    map
}

/// K8s manifest struct for deserialization
#[derive(serde::Deserialize)]
struct K8sManifest {
    kind: Option<String>,
    data: Option<HashMap<String, String>>,
}

/// Extract ConfigMap data from Kubernetes YAML (handles multi-document files)
fn extract_k8s_configmap_env(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    // Split on --- for multi-document YAML
    for doc_str in content.split("\n---") {
        let doc_str = doc_str.trim();
        if doc_str.is_empty() {
            continue;
        }

        match serde_yaml_bw::from_str::<K8sManifest>(doc_str) {
            Ok(manifest) => {
                if manifest.kind.as_deref() == Some("ConfigMap") {
                    if let Some(data) = manifest.data {
                        map.extend(data);
                    }
                }
            }
            Err(_) => {
                // Skip documents that don't parse as K8sManifest
                continue;
            }
        }
    }

    map
}
