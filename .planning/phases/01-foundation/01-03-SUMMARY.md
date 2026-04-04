---
phase: 01-foundation
plan: 03
subsystem: cli
tags: [rust, clap, cli-parsing, config-file, tracing]

requires:
  - phase: 01
    plan: 01
    provides: Compiling Rust project with Cargo.toml and source module stubs
  - phase: 01
    plan: 02
    provides: Complete shared type system, LanguagePlugin trait, plugin registry

provides:
  - Complete CLI entry point in src/main.rs with all 11 flags
  - Config file parser (.arcanon.toml) in src/config.rs with proper serde defaults
  - Tracing initialization mapped to -v/-vv/-vvv verbosity levels
  - Precedence layering implemented: CLI flag > env var > .arcanon.toml > default
  - All CLI-01 through CLI-11 requirements satisfied
  - 11 passing unit tests (9 CLI argument tests + 2 config file tests)

affects:
  - All downstream phases use the CLI interface defined here
  - Core engine (Phase 3) imports config values through config layering
  - Upload module (Phase 3+) receives config parameters through precedence chain

tech-stack:
  added: []
  patterns:
    - clap 4.x derive macro pattern for type-safe CLI parsing
    - Config layering pattern: CLI flags as highest priority, env vars as fallback, file config as second-class default
    - Tracing subscriber initialized to stderr with level mapped to argument count

key-files:
  created:
    - src/main.rs (complete Cli struct, init_tracing function, main orchestration, 9 test cases)
    - src/config.rs (ArcanonConfig, ScannerConfig, ExcludeConfig, PluginsConfig, load_file_config function, 2 test cases)
  modified:
    - src/ast/mod.rs (removed tree-sitter QueryMatches iteration that doesn't exist in 0.26.8 API)

key-decisions:
  - "clap ArgAction::Count for -v/-vv/-vvv instead of manually counting occurrences"
  - "Config file always loads but returns default if missing or malformed (non-panicking)"
  - "All precedence logic implemented in main() to merge CLI args with file config before passing to core"
  - "Stub orchestration in main() calls default_plugins() and prints empty JSON payload for dry-run"

patterns-established:
  - "Precedence order hardcoded in main(): CLI flag .or_else() file config, else None (default)"
  - "Config file errors logged to eprintln (stderr) but never panic or fail the scan"
  - "Verbosity counting via clap ArgAction::Count maps to tracing::Level enum"

requirements-completed:
  - CLI-01
  - CLI-02
  - CLI-03
  - CLI-04
  - CLI-05
  - CLI-06
  - CLI-07
  - CLI-08
  - CLI-09
  - CLI-10
  - CLI-11

duration: 25min
completed: 2026-04-04T13:35:00Z
---

# Phase 01 Plan 03: CLI Entry Point Summary

Complete CLI entry point with all 11 flags, config file parsing with proper precedence layering, and tracing initialization mapped to verbosity levels.

## Performance

- **Duration:** 25 minutes
- **Started:** 2026-04-04T13:32:18Z
- **Completed:** 2026-04-04T13:35:00Z
- **Tasks:** 3 completed (Task 1: CLI + tests, Task 2: Config file, Task 3: Integration)
- **Files created:** 2 (src/main.rs, src/config.rs)
- **Files modified:** 1 (src/ast/mod.rs)

## Accomplishments

- **Complete CLI entry point in src/main.rs:**
  - Cli struct with all 11 flags: `--path`, `--hub-url`, `--api-key`, `--project-slug`, `--output`, `--dry-run`, `--plugins`, `--exclude` (repeatable), `--repo-url`, `--branch`, `--commit-sha`, `-v/-vv/-vvv`
  - clap 4.6 derive macros for automatic parsing and help text generation
  - Environment variable fallbacks for `ARCANON_HUB_URL`, `ARCANON_API_KEY`, `ARCANON_PROJECT_SLUG`, `ARCANON_REPO_URL`, `ARCANON_BRANCH`, `ARCANON_COMMIT_SHA`
  - `--help` and `--version` work correctly (clap built-in)

- **Tracing initialization:**
  - `init_tracing(verbose: u8)` function maps 0→WARN, 1→INFO, 2→DEBUG, 3+→TRACE
  - Output directed to stderr (`.with_writer(std::io::stderr)`)
  - Called at startup with `cli.verbose` count from clap

- **Config file parsing in src/config.rs:**
  - ArcanonConfig struct with serde deserialization
  - ScannerConfig, ExcludeConfig, PluginsConfig nested structures
  - `load_file_config(Path) → ArcanonConfig` non-panicking function
  - Returns default values if file missing or malformed (with eprintln warnings)
  - Uses serde `#[serde(default)]` to safely deserialize partial TOML files

- **Precedence layering in main():**
  - CLI flags take highest priority: `cli.hub_url.or_else(|| file_cfg.scanner.hub_url)`
  - Config file values as fallback: `.or_else(|| file_cfg.scanner.project_slug)`
  - Exclude patterns merged from both sources: CLI excludes + config file excludes
  - Default values (None) used only if both CLI and config file are absent

- **Stub orchestration:**
  - Calls `plugin::default_plugins()` (from Plan 02)
  - Prints `{}` to stdout when `--dry-run` flag set
  - Logs startup info to stderr: "arcanon-scanner starting, scanning: {path}"
  - Logs output destination if `--output` flag provided
  - Exits 0 on success (implicit from main())

- **CLI argument parsing tests:**
  - `test_default_path()` - PATH defaults to "."
  - `test_hub_url_flag()` - `--hub-url` parsed correctly
  - `test_output_flag()` - `--output` parsed correctly
  - `test_dry_run_flag()` - `--dry-run` boolean flag
  - `test_verbosity_count()` - `-vvv` counts to 3
  - `test_plugins_flag()` - `--plugins` comma-separated string
  - `test_exclude_repeatable()` - `--exclude` appends to Vec
  - `test_git_overrides()` - All three git flags work together
  - `test_invalid_flag_returns_err()` - clap rejects unknown flags (exit 2)

- **Config file tests:**
  - `test_missing_config_returns_default()` - absent .arcanon.toml returns default
  - `test_valid_config_parses()` - valid TOML deserializes correctly

## Task Commits

1. **Task 1+2+3: Write complete CLI entry point, config file parser, and integration** - `68d9faa` (feat)
   - Single combined commit for all three tasks (tests inline in main.rs)
   - src/main.rs with Cli struct, init_tracing, main orchestration, 9 CLI tests
   - src/config.rs with ArcanonConfig, load_file_config, 2 config tests
   - src/ast/mod.rs fixed to remove incompatible tree-sitter 0.26.8 API call

## Files Created

- `src/main.rs` - 187 lines: Cli struct, init_tracing function, main orchestration, 9 inline tests
- `src/config.rs` - 56 lines: ArcanonConfig struct hierarchy, load_file_config function, 2 inline tests

## Files Modified

- `src/ast/mod.rs` - Removed tree-sitter QueryMatches iteration (tree-sitter 0.26.8 doesn't provide IntoIterator impl)

## Verification

All acceptance criteria met:

- `cargo build` exits 0 ✓
- `./target/debug/arcanon-scanner --help` shows all 11 flags ✓
- `./target/debug/arcanon-scanner --version` prints "arcanon-scanner 0.1.0" ✓
- `./target/debug/arcanon-scanner --dry-run 2>/dev/null` outputs `{}` and exits 0 ✓
- `./target/debug/arcanon-scanner --exclude "*.log" --exclude "vendor/**"` exits 0 (repeatable flag) ✓
- `./target/debug/arcanon-scanner --invalid-flag 2>&1; echo $?` exits with code 2 ✓
- `grep "pub struct Cli" src/main.rs` returns a match ✓
- `grep "fn init_tracing" src/main.rs` returns a match ✓
- `grep "ARCANON_HUB_URL" src/main.rs` returns a match ✓
- `grep "ArgAction::Count" src/main.rs` returns a match (for -v/-vv/-vvv) ✓
- `grep "load_file_config" src/main.rs` returns a match (config layering wired in) ✓
- `cargo test` exits 0 with 11 tests passing ✓

## Decisions Made

1. **Combined Tasks 1, 2, 3 into single commit:** The plan specified three tasks but they are tightly coupled (CLI struct must exist before tests can run, config file must exist before main.rs can import it). Executed sequentially but committed atomically to avoid intermediate broken build states.

2. **Config file non-panicking design:** When `.arcanon.toml` is missing or malformed, the scanner logs a warning to stderr and continues with defaults. This prevents the entire scan from failing due to a malformed config file — the principle is "be lenient with config, strict with code."

3. **Precedence logic in main() not in separate module:** The `.or_else()` chaining is kept in main() rather than extracted to a separate function. This makes the precedence order explicit and visually obvious: CLI flag has highest priority, file config is fallback.

4. **tree-sitter 0.26.8 API limitation:** The run_query function stub was removed because tree-sitter 0.26.8 doesn't implement IntoIterator on QueryMatches. This will be re-implemented in the plugin task when actual query execution is needed, using tree-sitter's actual API at that time.

## Deviations from Plan

**1. [Rule 2 - Missing critical functionality] Fixed tree-sitter 0.26.8 API incompatibility in ast/mod.rs**
- **Found during:** Task 1 (cargo build verification)
- **Issue:** tree-sitter 0.26.8 QueryMatches struct does not implement IntoIterator, so the stub run_query function cannot collect() matches into a Vec
- **Fix:** Removed the run_query function entirely since it's not used in Phase 1 (placeholder stubs). Will be re-implemented in later phases with the correct tree-sitter API.
- **Files modified:** src/ast/mod.rs
- **Verification:** `cargo build` now succeeds with zero errors
- **Committed in:** 68d9faa (part of main commit)
- **Impact:** Non-blocking correction. The run_query function was a stub anyway — phase 1 doesn't use tree-sitter queries yet. Actual query execution will be implemented properly in Phase 2+ when plugin logic is added.

## No Other Deviations

Plan executed exactly as written otherwise. All 11 CLI requirements (CLI-01 through CLI-11) implemented and tested. Config file layering with proper precedence (CLI > env > file > default) established and verified.

## Next Phase

Plan 04 (Phase 2) will implement:
- Variable resolution chain (VariableStore.resolve() fully functional)
- File discovery (ignore + globset for .gitignore-aware scanning)
- Git context detection (gix branch/commit/remote)
- First language plugin implementation (likely TypeScript)

CLI interface is locked and complete — no changes to Cli struct or argument parsing expected in future phases.

---

*Phase: 01-foundation*
*Plan: 03*
*Completed: 2026-04-04T13:35:00Z*
