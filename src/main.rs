use clap::Parser;
use std::path::PathBuf;
use tracing::info;

mod ast;
mod config;
mod core;
mod discovery;
mod git;
mod libres;
mod ml;
mod patterns;
mod plugin;
mod types;
mod upload;
mod vars;
mod wrapper;

/// Static service topology scanner for Arcanon Hub
#[derive(Parser, Debug)]
#[command(name = "arcanon", version, about = "Static service topology scanner")]
pub struct Cli {
    /// Root directory to scan
    #[arg(default_value = ".")]
    pub path: PathBuf,

    /// Upload scan results to Arcanon Hub
    #[arg(long)]
    pub upload: bool,

    /// Hub API base URL (overrides ~/.arcanon/config.json and ARCANON_HUB_URL)
    #[arg(long, env = "ARCANON_HUB_URL")]
    pub hub_url: Option<String>,

    /// API key for upload (overrides ARCANON_API_KEY env var)
    #[arg(long, env = "ARCANON_API_KEY")]
    pub api_key: Option<String>,

    /// Project slug for grouping
    #[arg(long, env = "ARCANON_PROJECT_SLUG")]
    pub project_slug: Option<String>,

    /// Write payload JSON to file instead of default output
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Parse and print payload to stdout, don't write or upload
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

    /// Enable ML-based refinement of findings
    #[arg(long)]
    pub ml_refine: bool,
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
    let log_file = dirs::home_dir().and_then(|home| {
        let log_dir = home.join(".arcanon").join("logs");
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

    // Load project config (.arcanon.toml in repo root)
    let file_cfg = config::load_file_config(&cli.path);

    // Load global user config (~/.arcanon/config.json)
    let global_cfg = config::load_global_config();

    // Resolve hub_url: --hub-url flag / ARCANON_HUB_URL env > ~/.arcanon/config.json
    // (clap already merged the env var into cli.hub_url via `env = "ARCANON_HUB_URL"`)
    let hub_url = cli.hub_url.or(global_cfg.hub_url);

    // Resolve api_key: --api-key flag / ARCANON_API_KEY env
    // (clap already merged the env var into cli.api_key via `env = "ARCANON_API_KEY"`)
    let api_key = cli.api_key;

    // Validate upload prerequisites before doing any work
    if cli.upload {
        if hub_url.is_none() {
            eprintln!(
                "Error: --upload requires a hub URL.\n\
                 Set it via --hub-url, ARCANON_HUB_URL env var, or hub_url in ~/.arcanon/config.json"
            );
            std::process::exit(1);
        }
        if api_key.is_none() {
            eprintln!(
                "Error: --upload requires an API key.\n\
                 Set it via --api-key or ARCANON_API_KEY env var"
            );
            std::process::exit(1);
        }
    }

    let project_slug = cli
        .project_slug
        .or(file_cfg.scanner.project_slug)
        .unwrap_or_else(|| "default-project".to_string());

    let mut exclude = cli.exclude.clone();
    exclude.extend(file_cfg.scanner.exclude.paths.unwrap_or_default());

    info!("arcanon starting, scanning: {}", cli.path.display());

    // Build scanner config
    let scanner_config = core::scanner::ScannerConfig {
        root: cli.path.clone(),
        dry_run: cli.dry_run,
        output: cli.output.clone(),
        hub_url: hub_url.clone(),
        project_slug,
        plugin_filter: cli.plugins.clone(),
        exclude_patterns: exclude,
        service_overrides: file_cfg
            .services
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    core::merger::ServiceOverride {
                        name: v.name,
                        ignore: v.ignore,
                    },
                )
            })
            .collect(),
        git_overrides: core::scanner::GitOverrides {
            repo_url: cli.repo_url.clone(),
            branch: cli.branch.clone(),
            commit_sha: cli.commit_sha.clone(),
        },
        user_pattern_overrides: file_cfg.user_patterns,
        disabled_patterns: file_cfg.scanner.patterns.disabled,
        ml_refine: cli.ml_refine || file_cfg.scanner.ml_refine.unwrap_or(false),
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
    let payload = match rt.block_on(core::scanner::run(&scanner_config)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Scan failed: {}", e);
            std::process::exit(1);
        }
    };

    // --dry-run: print JSON to stdout and exit
    if scanner_config.dry_run {
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

    // Serialize payload once for file writing paths
    let json = match serde_json::to_string_pretty(&payload) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("Failed to serialize payload: {}", e);
            std::process::exit(1);
        }
    };

    // --output <FILE>: write to specified path
    if let Some(output_path) = &scanner_config.output {
        match std::fs::write(output_path, &json) {
            Ok(_) => info!("Payload written to {}", output_path.display()),
            Err(e) => {
                eprintln!("Failed to write output file: {}", e);
                std::process::exit(1);
            }
        }
    } else if !cli.upload {
        // Default: write {reponame}.json in repo root
        let resolved_root = cli.path.canonicalize().unwrap_or_else(|_| cli.path.clone());
        let repo_name = resolved_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("arcanon-scan");
        let output_path = cli.path.join(format!("{}.json", repo_name));
        match std::fs::write(&output_path, &json) {
            Ok(_) => info!("Payload written to {}", output_path.display()),
            Err(e) => {
                eprintln!("Failed to write {}: {}", output_path.display(), e);
                std::process::exit(1);
            }
        }
    }

    // --upload: POST to hub
    if cli.upload {
        let upload_config = upload::UploadConfig {
            hub_url: hub_url.unwrap(), // validated above
            api_key: api_key.unwrap(), // validated above
        };
        match rt.block_on(upload::upload(&payload, &upload_config)) {
            Ok(()) => info!("Upload complete"),
            Err(e) => {
                eprintln!("Upload failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    std::process::exit(0);
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
        let cli = Cli::try_parse_from(["arcanon", "--hub-url", "https://api.arcanon.dev"]).unwrap();
        assert_eq!(cli.hub_url.as_deref(), Some("https://api.arcanon.dev"));
    }

    #[test]
    fn test_upload_flag() {
        let cli = Cli::try_parse_from(["arcanon", "--upload"]).unwrap();
        assert!(cli.upload);
    }

    #[test]
    fn test_api_key_flag() {
        let cli = Cli::try_parse_from(["arcanon", "--api-key", "sk-test-123"]).unwrap();
        assert_eq!(cli.api_key.as_deref(), Some("sk-test-123"));
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
        let cli = Cli::try_parse_from(["arcanon", "--plugins", "openapi,typescript"]).unwrap();
        assert_eq!(cli.plugins.as_deref(), Some("openapi,typescript"));
    }

    #[test]
    fn test_exclude_repeatable() {
        let cli = Cli::try_parse_from(["arcanon", "--exclude", "*.log", "--exclude", "vendor/**"])
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
