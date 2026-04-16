//! # Memo - Command Memoization Tool
//!
//! Memo is a command-line tool that memoizes (caches) shell command execution results.
//! When you run a command through memo, it stores the stdout, stderr, and exit code.
//! Subsequent executions of the same command will replay the cached results instantly
//! without re-running the command.
//!
//! ## How It Works
//!
//! - **Cache Key**: SHA-256 hash of the command arguments and current working directory
//! - **Storage**: Each memoized command is stored in a subdirectory:
//!   - `<digest>/meta.json` - Metadata (command, exit code, timestamp)
//!   - `<digest>/stdout` - Captured stdout
//!   - `<digest>/stderr` - Captured stderr
//! - **Location**: `$XDG_CACHE_HOME/memo/` (defaults to `~/.cache/memo/`)
//!
//! ## Usage Examples
//!
//! ```bash
//! # First run executes the command
//! memo echo "Hello, World!"
//!
//! # Second run replays from cache (instant)
//! memo echo "Hello, World!"
//!
//! # Verbose mode shows cache hits/misses
//! memo -v ls -la /etc
//!
//! # Commands with different arguments create separate cache entries
//! memo echo "foo"
//! memo echo "bar"
//! ```
//!
//! ## Features
//!
//! - Preserves exact stdout, stderr, and exit codes
//! - Handles binary data correctly
//! - Streaming architecture for memory efficiency
//! - Atomic directory-based concurrency control (lock-free)
//! - Secure file permissions on Unix systems

mod cache;
mod constants;
mod digest;
mod error;
mod executor;
mod logger;
mod memo;

use cache::{
    cleanup_temp_dirs, commit_cache_dir, create_temp_cache_dir, ensure_cache_dir, get_cache_dir,
    is_memo_disabled, memo_complete, read_memo_metadata, stream_stderr, stream_stdout,
};
use chrono::Utc;
use clap::Parser;
use digest::compute_digest;
use error::Result;
use executor::{build_command_string, execute_and_stream, execute_direct};
use log::{debug, error, info};
use memo::Memo;
use std::fs;
use std::io::{self, Write};
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "memo",
    version = env!("VERGEN_GIT_DESCRIBE"),
    long_version = concat!(
        env!("VERGEN_GIT_DESCRIBE"),
        "\ncommit-id     ", env!("VERGEN_GIT_SHA"),
        "\ncommit-time   ", env!("VERGEN_GIT_COMMIT_TIMESTAMP"),
        "\nrustc-version ", env!("VERGEN_RUSTC_SEMVER"),
        "\ntarget-arch   ", env!("VERGEN_CARGO_TARGET_TRIPLE"),
    ),
)]
#[command(about = "Memoize shell command execution", long_about = None)]
#[command(after_help = "** SECURITY WARNING **\n\n\
    Memoization caches stdout/stderr to disk UNENCRYPTED. Do NOT use memo with commands \
that output sensitive information such as:\n\
    - Authentication tokens or API keys\n\
    - Passwords or credentials\n\
    - Private keys or certificates\n\
    - Personally identifiable information\n\n\
    Cached files are stored in ~/.cache/memo/ and may be accessible to other users on shared systems.\n\
    Use MEMO_DISABLE=1 to bypass caching for individual commands with sensitive output.")]
struct Cli {
    /// Print memoization information
    #[arg(short, long, action = clap::ArgAction::Count, help = "Increase verbosity (-v, -vv, -vvv)")]
    verbose: u8,

    /// Suppress all memo messages (even errors)
    #[arg(short, long, help = "Suppress all memo messages (even errors)")]
    quiet: bool,

    /// Cache expiration time (e.g. "1h 30m", "10s")
    #[arg(long, help = "Time to live for cache entry (e.g. \"1h 30m\")")]
    ttl: Option<String>,

    /// Environment variables to consider for the cache key
    #[arg(
        short,
        long,
        value_delimiter = ',',
        help = "Environment variables to consider for the cache key"
    )]
    env: Option<Vec<String>>,

    /// Purge all cache entries
    #[arg(long, conflicts_with_all = ["env", "command"], help = "Purge all cache entries")]
    purge: bool,

    /// Command to execute/memoize
    #[arg(
        trailing_var_arg = true,
        required_unless_present = "purge",
        allow_hyphen_values = true
    )]
    command: Vec<String>,
}

fn main() {
    let args = Cli::parse();

    let level = if args.quiet {
        log::LevelFilter::Off
    } else {
        match args.verbose {
            0 => log::LevelFilter::Error,
            1 => log::LevelFilter::Info,
            2 => log::LevelFilter::Debug,
            _ => log::LevelFilter::Trace,
        }
    };
    logger::init(level).expect("Failed to initialize logger");

    match run(args) {
        Ok(exit_code) => process::exit(exit_code),
        Err(e) => {
            error!("{}", e);
            process::exit(1);
        }
    }
}

fn run(args: Cli) -> Result<i32> {
    // Check if memoization is disabled
    if is_memo_disabled() {
        info!("disabled");

        // Convert Vec<String> to Vec<&str>
        let cmd_args: Vec<&str> = args.command.iter().map(|s| s.as_str()).collect();

        // Execute directly without caching
        let result = execute_direct(&cmd_args)?;
        return Ok(result.exit_code);
    }

    // Get cache directory
    let cache_dir = get_cache_dir()?;

    if args.purge {
        info!("purging cache at {}", cache_dir.display());
        crate::cache::purge_cache(&cache_dir)?;
        return Ok(0);
    }

    // Parse TTL if provided (fail fast before any real work)
    let parsed_ttl = match args.ttl {
        Some(ttl_str) => {
            let duration = humantime::parse_duration(&ttl_str)
                .map_err(|_| crate::error::MemoError::InvalidTtl(ttl_str.clone()))?;
            Some(duration)
        }
        None => None,
    };

    ensure_cache_dir(&cache_dir)?;

    // Clean up any orphaned temp directories from previous crashes
    cleanup_temp_dirs(&cache_dir)?;

    // Collect environment variables for cache key
    let mut env_vars = std::collections::BTreeMap::new();
    if let Some(vars) = args.env {
        debug!(
            "Capturing {} environment variables for cache key",
            vars.len()
        );
        for var in vars {
            let value = std::env::var(&var).ok();
            log::trace!("Captured env var {} = {:?}", var, value);
            env_vars.insert(var.clone(), value);
        }
    }

    // Build command string for display and compute digest from argv.
    let command_string = build_command_string(&args.command);
    debug!("Computing digest for command: {:?}", args.command);
    let digest = compute_digest(&args.command, &env_vars)?;

    // Check if memo exists
    if memo_complete(&cache_dir, &digest) {
        // Read metadata
        match read_memo_metadata(&cache_dir, &digest) {
            Ok(memo) => {
                if memo.is_expired() {
                    info!("expired `{command_string}` => {digest}");
                    // Delete the expired directory to avoid DirectoryNotEmpty errors on next commit
                    let digest_dir = cache_dir.join(&digest);
                    if let Err(e) = std::fs::remove_dir_all(&digest_dir) {
                        log::warn!("Failed to delete expired cache directory: {}", e);
                    }
                } else {
                    // Cache hit - replay
                    info!("hit `{command_string}` => {digest}");

                    // Stream output to stdout/stderr
                    stream_stdout(&cache_dir, &digest, io::stdout())?;
                    stream_stderr(&cache_dir, &digest, io::stderr())?;

                    // Exit with stored exit code
                    return Ok(memo.exit_code);
                }
            }
            Err(e) => {
                log::warn!("Failed to read metadata for {}: {}", digest, e);
            }
        }
    }

    // Cache miss - execute and memoize
    info!("miss `{command_string}` => {digest}");

    let now = Utc::now();
    let timestamp = now.to_rfc3339();

    let expires_at = parsed_ttl.map(|duration| {
        let chrono_duration =
            chrono::Duration::from_std(duration).expect("Duration is too large for chrono");
        (now + chrono_duration).to_rfc3339()
    });

    // Create a temp directory for this process to write cache files
    let mut temp_dir = create_temp_cache_dir(&cache_dir, &digest)?;
    let (json_path, out_path, err_path) = temp_dir.get_paths();

    // Convert Vec<String> to Vec<&str>
    let cmd_args: Vec<&str> = args.command.iter().map(|s| s.as_str()).collect();

    // Execute command and stream to files AND console simultaneously
    let result = execute_and_stream(&cmd_args, &out_path, &err_path)?;

    // Report any file write errors
    if let Some(path) = &result.stdout_error {
        error!("could not write {}", path.display());
    }
    if let Some(path) = &result.stderr_error {
        error!("could not write {}", path.display());
    }

    // Create memo metadata
    let memo = Memo {
        cmd: args.command.clone(),
        env: env_vars,
        exit_code: result.exit_code,
        timestamp,
        expires_at,
        digest: digest.clone(),
    };

    // Write metadata to JSON
    let json = serde_json::to_string_pretty(&memo)?;
    {
        let mut f = fs::File::create(&json_path)?;
        f.write_all(json.as_bytes())?;
    }

    // Atomically commit the temp directory to the final location
    // If another process already committed, that's fine - we just clean up
    let committed = commit_cache_dir(&mut temp_dir, &cache_dir, &digest)?;

    if committed {
        debug!("committed temp dir {}", temp_dir.path.display());
    } else {
        debug!("dropping temp dir {}", temp_dir.path.display());
    }

    // Exit with command's exit code (output already streamed to console)
    Ok(result.exit_code)
}
