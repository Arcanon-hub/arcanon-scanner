use clap::Parser;
use std::path::PathBuf;
use tracing::info;

mod ast;
mod config;
mod core;
mod discovery;
mod git;
mod libres;
mod patterns;
mod plugin;
mod types;
mod upload;
mod vars;

/// Static service topology scanner for Arcanon Hub
#[derive(Parser, Debug)]
#[command(
    name = "arcanon",
    version,
    about = "Static service topology scanner"
)]
pub struct Cli {
    /// Root directory to scan
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Hub API endpoint
    #[arg(long, env = "ARCANON_HUB_URL")]
    pub hub_url: Option<String>,

    /// API key for upload
    #[arg(long, env = "ARCANON_API_KEY")]
    pub api_key: Option<String>,

    /// Project slug for grouping
    #[arg(long, env = "ARCANON_PROJECT_SLUG")]
    pub project_slug: Option<String>,

    /// Write payload JSON to file instead of uploading
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Parse and print payload, don't upload
    #[arg(long)]
    pub dry_run: bool,

    /// Comma-separated plugin filter
    #[arg(long)]
    pub plugins: Option<String>,

    /// Glob patterns to exclude (repeatable)
    #[arg(long, action = clap::ArgAction::Append)]
    pub exclude: Vec<String>,

    /// Override git remote detection
    #[arg(long, env = "ARCANON_REPO_URL")]
    pub repo_url: Option<String>,

    /// Override branch detection
    #[arg(long, env = "ARCANON_BRANCH")]
    pub branch: Option<String>,

    /// Override commit SHA detection
    #[arg(long, env = "ARCANON_COMMIT_SHA")]
    pub commit_sha: Option<String>,

    /// Increase log verbosity (repeatable: -v info, -vv debug, -vvv trace)
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

/// Initialize tracing: stderr at user-selected level + file at DEBUG always.
/// Log file: ~/.arcanon/logs/{repo}-{datetime}.log (one per scan, never overwritten).
fn init_tracing(verbose: u8, scan_root: &std::path::Path) {
    let level = match verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };

    // Derive repo name from scan root directory (resolve "." to actual dir name)
    let resolved_root = scan_root
        .canonicalize()
        .unwrap_or_else(|_| scan_root.to_path_buf());
    let repo_name = resolved_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    // Timestamp for unique log file name
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let log_filename = format!("{}-{}.log", repo_name, timestamp);

    // Always write debug-level log to ~/.arcanon/logs/
    let log_file = std::env::var("HOME").ok().and_then(|home| {
        let log_dir = std::path::PathBuf::from(&home).join(".arcanon").join("logs");
        let _ = std::fs::create_dir_all(&log_dir);
        std::fs::File::create(log_dir.join(&log_filename)).ok()
    });

    if let Some(file) = log_file {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;
        use tracing_subscriber::Layer;

        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(tracing_subscriber::EnvFilter::new(level));

        let file_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .with_filter(tracing_subscriber::EnvFilter::new("debug"));

        tracing_subscriber::registry()
            .with(stderr_layer)
            .with(file_layer)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_max_level(match level {
                "warn" => tracing::Level::WARN,
                "info" => tracing::Level::INFO,
                "debug" => tracing::Level::DEBUG,
                _ => tracing::Level::TRACE,
            })
            .with_writer(std::io::stderr)
            .init();
    }
}

fn main() {
    let cli = Cli::parse();

    // Initialize logging
    init_tracing(cli.verbose, &cli.path);

    // Load config file (.arcanon.toml)
    let file_cfg = config::load_file_config(&cli.path);

    // Apply precedence: CLI flag > env var > .arcanon.toml > default
    let hub_url = cli
        .hub_url
        .or(file_cfg.scanner.hub_url)
        .unwrap_or_else(|| "https://hub.arcanon.dev".to_string());
    let api_key = std::env::var("ARCANON_API_KEY")
        .unwrap_or_else(|_| "placeholder-key".to_string());
    let project_slug = cli
        .project_slug
        .or(file_cfg.scanner.project_slug)
        .unwrap_or_else(|| "default-project".to_string());

    let mut exclude = cli.exclude.clone();
    exclude.extend(file_cfg.scanner.exclude.paths.unwrap_or_default());

    // Log startup
    info!("arcanon starting, scanning: {}", cli.path.display());

    // Log output destination if specified
    if let Some(ref p) = cli.output {
        info!("Output will be written to: {}", p.display());
    }

    // Build scanner config
    let scanner_config = core::scanner::ScannerConfig {
        root: cli.path.clone(),
        dry_run: cli.dry_run,
        output: cli.output.clone(),
        hub_url,
        api_key,
        project_slug,
        plugin_filter: cli.plugins.clone(),
        exclude_patterns: exclude,
        service_overrides: std::collections::HashMap::new(), // TODO: load from .arcanon.toml [services]
        git_overrides: core::scanner::GitOverrides {
            repo_url: cli.repo_url.clone(),
            branch: cli.branch.clone(),
            commit_sha: cli.commit_sha.clone(),
        },
        user_pattern_overrides: file_cfg.user_patterns,
        disabled_patterns: file_cfg.scanner.patterns.disabled,
    };

    // Create tokio runtime for async scanner
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to create tokio runtime: {}", e);
            std::process::exit(1);
        }
    };

    // Run the scanner
    match rt.block_on(core::scanner::run(&scanner_config)) {
        Ok(payload) => {
            if scanner_config.dry_run {
                // --dry-run: print payload to stdout and exit 0
                match serde_json::to_string_pretty(&payload) {
                    Ok(json) => {
                        println!("{}", json);
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("Failed to serialize payload: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            if let Some(output_path) = &scanner_config.output {
                // --output <FILE>: write to file and exit 0
                match serde_json::to_string_pretty(&payload) {
                    Ok(json) => match std::fs::write(output_path, json) {
                        Ok(_) => {
                            info!("Payload written to {}", output_path.display());
                            std::process::exit(0);
                        }
                        Err(e) => {
                            eprintln!("Failed to write output file: {}", e);
                            std::process::exit(1);
                        }
                    },
                    Err(e) => {
                        eprintln!("Failed to serialize payload: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            // Default: upload to hub using the same runtime
            let upload_config = upload::UploadConfig {
                hub_url: scanner_config.hub_url.clone(),
                api_key: scanner_config.api_key.clone(),
            };

            match rt.block_on(upload::upload(&payload, &upload_config)) {
                Ok(()) => {
                    info!("Scan complete");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("Upload failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("Scan failed: {}", e);
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_path() {
        let cli = Cli::try_parse_from(["arcanon"]).unwrap();
        assert_eq!(cli.path.to_str().unwrap(), ".");
    }

    #[test]
    fn test_hub_url_flag() {
        let cli = Cli::try_parse_from(["arcanon", "--hub-url", "https://hub.arcanon.dev"])
            .unwrap();
        assert_eq!(cli.hub_url.as_deref(), Some("https://hub.arcanon.dev"));
    }

    #[test]
    fn test_output_flag() {
        let cli = Cli::try_parse_from(["arcanon", "--output", "result.json"]).unwrap();
        assert!(cli.output.is_some());
    }

    #[test]
    fn test_dry_run_flag() {
        let cli = Cli::try_parse_from(["arcanon", "--dry-run"]).unwrap();
        assert!(cli.dry_run);
    }

    #[test]
    fn test_verbosity_count() {
        let cli = Cli::try_parse_from(["arcanon", "-vvv"]).unwrap();
        assert_eq!(cli.verbose, 3);
    }

    #[test]
    fn test_plugins_flag() {
        let cli =
            Cli::try_parse_from(["arcanon", "--plugins", "openapi,typescript"]).unwrap();
        assert_eq!(cli.plugins.as_deref(), Some("openapi,typescript"));
    }

    #[test]
    fn test_exclude_repeatable() {
        let cli = Cli::try_parse_from([
            "arcanon",
            "--exclude",
            "*.log",
            "--exclude",
            "vendor/**",
        ])
        .unwrap();
        assert_eq!(cli.exclude.len(), 2);
        assert!(cli.exclude.contains(&"*.log".to_string()));
        assert!(cli.exclude.contains(&"vendor/**".to_string()));
    }

    #[test]
    fn test_git_overrides() {
        let cli = Cli::try_parse_from([
            "arcanon",
            "--repo-url",
            "https://github.com/example/repo",
            "--branch",
            "main",
            "--commit-sha",
            "abc123",
        ])
        .unwrap();
        assert_eq!(
            cli.repo_url.as_deref(),
            Some("https://github.com/example/repo")
        );
        assert_eq!(cli.branch.as_deref(), Some("main"));
        assert_eq!(cli.commit_sha.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_invalid_flag_returns_err() {
        let result = Cli::try_parse_from(["arcanon", "--nonexistent-flag"]);
        assert!(result.is_err());
    }
}
