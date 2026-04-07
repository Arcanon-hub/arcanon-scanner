use url::Url;

/// Returns (protocol, hostname) for a URL-like value, or None if not URL-like.
/// Skips template variables (hostname contains '{').
pub fn parse_url_value(val: &str) -> Option<(String, String)> {
    let url = Url::parse(val).ok()?;
    let hostname = url.host_str()?.to_string();
    if hostname.is_empty() || hostname.contains('{') {
        return None;
    }
    let protocol = scheme_to_protocol(url.scheme());
    Some((protocol, hostname))
}

/// Map URL scheme to canonical protocol string.
/// Returns the scheme as-is for unknown schemes (free string, no enum per CLAUDE.md).
pub fn scheme_to_protocol(scheme: &str) -> String {
    match scheme {
        "postgres" | "postgresql" => "postgresql",
        "mysql" | "mariadb" => "mysql",
        "redis" | "rediss" => "redis",
        "amqp" | "amqps" => "amqp",
        "kafka" => "kafka",
        "mongodb" | "mongodb+srv" => "mongodb",
        "http" | "https" => "http",
        "grpc" | "grpcs" => "grpc",
        other => other,
    }
    .to_string()
}

/// Returns true if the env key matches a connection pattern.
/// Suffix patterns: *_URL, *_HOST, *_ENDPOINT, *_ADDR, *_DSN
/// Exact names: DATABASE_URL, REDIS_URL, AMQP_URL, KAFKA_BROKERS
pub fn is_connection_key(key: &str) -> bool {
    const SUFFIXES: &[&str] = &["_URL", "_HOST", "_ENDPOINT", "_ADDR", "_DSN"];
    const EXACT: &[&str] = &["DATABASE_URL", "REDIS_URL", "AMQP_URL", "KAFKA_BROKERS"];
    EXACT.contains(&key) || SUFFIXES.iter().any(|suf| key.ends_with(suf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url_value_postgres() {
        let result = parse_url_value("postgres://db.host/mydb");
        assert_eq!(
            result,
            Some(("postgresql".to_string(), "db.host".to_string()))
        );
    }

    #[test]
    fn test_parse_url_value_redis() {
        let result = parse_url_value("redis://cache.internal:6379");
        assert_eq!(
            result,
            Some(("redis".to_string(), "cache.internal".to_string()))
        );
    }

    #[test]
    fn test_parse_url_value_template_skipped() {
        let result = parse_url_value("https://{server}/v2");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_url_value_non_url() {
        let result = parse_url_value("not-a-url");
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_url_value_empty() {
        let result = parse_url_value("");
        assert_eq!(result, None);
    }

    #[test]
    fn test_scheme_to_protocol_known() {
        assert_eq!(scheme_to_protocol("postgres"), "postgresql");
        assert_eq!(scheme_to_protocol("postgresql"), "postgresql");
        assert_eq!(scheme_to_protocol("mysql"), "mysql");
        assert_eq!(scheme_to_protocol("mariadb"), "mysql");
        assert_eq!(scheme_to_protocol("redis"), "redis");
        assert_eq!(scheme_to_protocol("rediss"), "redis");
        assert_eq!(scheme_to_protocol("amqp"), "amqp");
        assert_eq!(scheme_to_protocol("mongodb"), "mongodb");
        assert_eq!(scheme_to_protocol("http"), "http");
        assert_eq!(scheme_to_protocol("https"), "http");
        assert_eq!(scheme_to_protocol("grpc"), "grpc");
        assert_eq!(scheme_to_protocol("grpcs"), "grpc");
    }

    #[test]
    fn test_scheme_to_protocol_amqps() {
        assert_eq!(scheme_to_protocol("amqps"), "amqp");
    }

    #[test]
    fn test_scheme_to_protocol_unknown() {
        assert_eq!(scheme_to_protocol("ftp"), "ftp");
    }

    #[test]
    fn test_is_connection_key_exact_database_url() {
        assert!(is_connection_key("DATABASE_URL"));
    }

    #[test]
    fn test_is_connection_key_exact_kafka_brokers() {
        assert!(is_connection_key("KAFKA_BROKERS"));
    }

    #[test]
    fn test_is_connection_key_suffixes() {
        assert!(is_connection_key("MY_SERVICE_HOST"));
        assert!(is_connection_key("API_URL"));
        assert!(is_connection_key("SERVICE_ENDPOINT"));
        assert!(is_connection_key("DB_ADDR"));
        assert!(is_connection_key("POSTGRES_DSN"));
    }

    #[test]
    fn test_is_connection_key_non_match_app_name() {
        assert!(!is_connection_key("APP_NAME"));
    }

    #[test]
    fn test_is_connection_key_non_match_port() {
        assert!(!is_connection_key("PORT"));
    }
}
