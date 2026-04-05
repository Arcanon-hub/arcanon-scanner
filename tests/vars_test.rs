use arcanon::vars::{build_variable_store, VariableStore};
use std::path::PathBuf;

fn write_file(dir: &std::path::Path, name: &str, content: &[u8]) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn test_env_file_priority_order() {
    let dir = tempfile::tempdir().unwrap();
    let f1 = write_file(dir.path(), ".env", b"DB_HOST=dev-host\n");
    let f2 = write_file(dir.path(), ".env.production", b"DB_HOST=prod-host\n");
    let files = vec![f1, f2];

    let store = build_variable_store(dir.path(), &files);
    assert_eq!(
        store.resolve("DB_HOST"),
        Some("prod-host"),
        ".env.production should win over .env for the same key"
    );
}

#[test]
fn test_env_file_local_overrides_base() {
    let dir = tempfile::tempdir().unwrap();
    let f1 = write_file(dir.path(), ".env", b"SECRET=base-secret\n");
    let f2 = write_file(dir.path(), ".env.local", b"SECRET=local-secret\n");
    let files = vec![f1, f2];

    let store = build_variable_store(dir.path(), &files);
    assert_eq!(
        store.resolve("SECRET"),
        Some("local-secret"),
        ".env.local should override .env for same key"
    );
}

#[test]
fn test_env_quoted_values() {
    let dir = tempfile::tempdir().unwrap();
    let content = b"QUOTED_DOUBLE=\"hello world\"\nQUOTED_SINGLE='goodbye'\n";
    let f = write_file(dir.path(), ".env", content);
    let store = build_variable_store(dir.path(), &[f]);

    assert_eq!(
        store.resolve("QUOTED_DOUBLE"),
        Some("hello world"),
        "double-quoted value should be stored without quotes"
    );
    assert_eq!(
        store.resolve("QUOTED_SINGLE"),
        Some("goodbye"),
        "single-quoted value should be stored without quotes"
    );
}

#[test]
fn test_env_export_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let f = write_file(dir.path(), ".env", b"export MY_VAR=exported-value\n");
    let store = build_variable_store(dir.path(), &[f]);
    assert_eq!(
        store.resolve("MY_VAR"),
        Some("exported-value"),
        "export prefix should be stripped"
    );
}

#[test]
fn test_compose_list_form() {
    let dir = tempfile::tempdir().unwrap();
    let content = b"
services:
  api:
    image: myapp
    environment:
      - API_KEY=secret123
      - DB_URL=postgres://localhost/mydb
";
    let f = write_file(dir.path(), "docker-compose.yml", content);
    let store = build_variable_store(dir.path(), &[f]);
    assert_eq!(
        store.resolve("API_KEY"),
        Some("secret123"),
        "list-form compose environment should be parsed"
    );
    assert_eq!(store.resolve("DB_URL"), Some("postgres://localhost/mydb"));
}

#[test]
fn test_compose_map_form() {
    let dir = tempfile::tempdir().unwrap();
    let content = b"
services:
  worker:
    image: myworker
    environment:
      WORKER_THREADS: '4'
      QUEUE_URL: amqp://rabbitmq:5672
";
    let f = write_file(dir.path(), "docker-compose.yml", content);
    let store = build_variable_store(dir.path(), &[f]);
    assert_eq!(
        store.resolve("WORKER_THREADS"),
        Some("4"),
        "map-form compose environment should be parsed"
    );
    assert_eq!(store.resolve("QUEUE_URL"), Some("amqp://rabbitmq:5672"));
}

#[test]
fn test_k8s_configmap() {
    let dir = tempfile::tempdir().unwrap();
    let content = b"
apiVersion: v1
kind: ConfigMap
metadata:
  name: app-config
data:
  REDIS_HOST: redis-master
  REDIS_PORT: '6379'
";
    std::fs::create_dir_all(dir.path().join("k8s")).unwrap();
    let f = write_file(&dir.path().join("k8s"), "configmap.yml", content);
    let store = build_variable_store(dir.path(), &[f]);
    assert_eq!(
        store.resolve("REDIS_HOST"),
        Some("redis-master"),
        "k8s ConfigMap data should be in variable store"
    );
    assert_eq!(store.resolve("REDIS_PORT"), Some("6379"));
}

#[test]
fn test_k8s_multi_document() {
    let dir = tempfile::tempdir().unwrap();
    let content = b"
apiVersion: v1
kind: ConfigMap
metadata:
  name: config-a
data:
  KEY_A: value-a
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: config-b
data:
  KEY_B: value-b
";
    std::fs::create_dir_all(dir.path().join("manifests")).unwrap();
    let f = write_file(&dir.path().join("manifests"), "multi.yml", content);
    let store = build_variable_store(dir.path(), &[f]);
    assert_eq!(
        store.resolve("KEY_A"),
        Some("value-a"),
        "first ConfigMap in multi-doc YAML should be extracted"
    );
    assert_eq!(
        store.resolve("KEY_B"),
        Some("value-b"),
        "second ConfigMap in multi-doc YAML should be extracted"
    );
}

#[test]
fn test_resolve_priority_env_over_compose() {
    let dir = tempfile::tempdir().unwrap();
    let env_f = write_file(dir.path(), ".env", b"SHARED_KEY=from-env\n");
    let compose_f = write_file(
        dir.path(),
        "docker-compose.yml",
        b"services:\n  app:\n    environment:\n      SHARED_KEY: from-compose\n",
    );
    let store = build_variable_store(dir.path(), &[env_f, compose_f]);
    assert_eq!(
        store.resolve("SHARED_KEY"),
        Some("from-env"),
        "env_files should have higher priority than compose_env"
    );
}

#[test]
fn test_resolve_missing_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = build_variable_store(dir.path(), &[]);
    assert_eq!(store.resolve("NONEXISTENT_KEY"), None);
}

#[test]
fn test_resolve_to_target_http_with_port() {
    let dir = tempfile::tempdir().unwrap();
    let f = write_file(
        dir.path(),
        ".env",
        b"USER_SERVICE_URL=http://user-service:3000/api/v1\n",
    );
    let store = build_variable_store(dir.path(), &[f]);
    let target = store
        .resolve_to_target("USER_SERVICE_URL")
        .expect("should resolve to target");
    assert_eq!(target.hostname, "user-service");
    assert_eq!(target.port, Some(3000));
}

#[test]
fn test_resolve_to_target_no_port() {
    let dir = tempfile::tempdir().unwrap();
    let f = write_file(dir.path(), ".env", b"API_URL=https://api.example.com/v1\n");
    let store = build_variable_store(dir.path(), &[f]);
    let target = store
        .resolve_to_target("API_URL")
        .expect("should resolve to target");
    assert_eq!(target.hostname, "api.example.com");
    assert_eq!(target.port, None);
}

#[test]
fn test_resolve_to_target_invalid_url() {
    let dir = tempfile::tempdir().unwrap();
    let f = write_file(dir.path(), ".env", b"SERVICE_NAME=order-service\n");
    let store = build_variable_store(dir.path(), &[f]);
    // A plain service name without scheme is not a URL
    let target = store.resolve_to_target("SERVICE_NAME");
    assert!(
        target.is_none(),
        "plain string without scheme should return None from resolve_to_target"
    );
}
