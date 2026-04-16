//! Cache management for memoized command execution
//!
//! This module handles all file I/O operations for the memo cache, including:
//! - Cache directory management
//! - File path generation
//! - Memo metadata storage and retrieval
//! - Output streaming
//! - Atomic directory-based concurrency control
//!
//! # Storage Structure
//!
//! Each memoized command is stored in a subdirectory named by its digest:
//! - `<digest>/meta.json` - Metadata (command, exit code, timestamp, digest)
//! - `<digest>/stdout` - Raw stdout bytes
//! - `<digest>/stderr` - Raw stderr bytes
//!
//! # Concurrency Strategy
//!
//! Uses atomic directory rename for lock-free concurrent writes:
//! 1. Each process writes to a temp directory: `<digest>.tmp.<pid>.<timestamp>/`
//! 2. After completion, atomically renames temp dir to `<digest>/`
//! 3. First rename wins; losers detect the existing directory and clean up
//! 4. Orphaned temp directories are cleaned up on startup

use crate::constants::CACHE_DIR_PERMISSIONS;
use crate::error::{MemoError, Result};
use crate::memo::Memo;
use chrono::Utc;
use std::fs::{self, File};
use std::io::{self, copy};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Check if memoization is disabled via environment variable
///
/// Returns `true` if `MEMO_DISABLE=1`, otherwise `false`.
///
/// # Examples
///
/// ```no_run
/// # use memo::cache::is_memo_disabled;
/// if is_memo_disabled() {
///     println!("Memoization is disabled");
/// }
/// ```
pub fn is_memo_disabled() -> bool {
    std::env::var("MEMO_DISABLE")
        .map(|val| val == "1")
        .unwrap_or(false)
}

/// Get the cache directory path
///
/// Respects `$XDG_CACHE_HOME` environment variable, falling back to `~/.cache`.
///
/// # Examples
///
/// ```no_run
/// # use memo::cache::get_cache_dir;
/// let cache_dir = get_cache_dir().expect("Failed to get cache directory");
/// println!("Cache directory: {:?}", cache_dir);
/// ```
pub fn get_cache_dir() -> Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg)
    } else {
        dirs::home_dir()
            .ok_or(MemoError::HomeNotFound)?
            .join(".cache")
    };
    Ok(base.join("memo"))
}

/// Ensure the cache directory exists with appropriate permissions
///
/// Creates the directory if it doesn't exist, and sets secure permissions (0o700)
/// on Unix systems.
pub fn ensure_cache_dir(cache_dir: &Path) -> io::Result<()> {
    fs::create_dir_all(cache_dir)?;

    #[cfg(unix)]
    {
        let perm = fs::Permissions::from_mode(CACHE_DIR_PERMISSIONS);
        let _ = fs::set_permissions(cache_dir, perm);
    }

    Ok(())
}

/// Check if a memo is complete (the digest directory exists with all three files)
///
/// Returns `true` if the `<digest>/` directory exists with `meta.json`, `stdout`, and `stderr`.
pub fn memo_complete(cache_dir: &Path, digest: &str) -> bool {
    let digest_dir = cache_dir.join(digest);
    digest_dir.join("meta.json").exists()
        && digest_dir.join("stdout").exists()
        && digest_dir.join("stderr").exists()
}

/// Get paths to the three cache files within a digest directory
pub fn get_cache_paths_in_dir(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let json_path = dir.join("meta.json");
    let out_path = dir.join("stdout");
    let err_path = dir.join("stderr");
    (json_path, out_path, err_path)
}

/// Get paths to the cache files for a digest (convenience wrapper)
#[cfg(test)]
pub fn get_cache_paths(cache_dir: &Path, digest: &str) -> (PathBuf, PathBuf, PathBuf) {
    let digest_dir = cache_dir.join(digest);
    get_cache_paths_in_dir(&digest_dir)
}

/// Create a secure directory with appropriate permissions
fn create_secure_dir(path: &Path) -> io::Result<()> {
    fs::create_dir(path)?;

    #[cfg(unix)]
    {
        let perm = fs::Permissions::from_mode(CACHE_DIR_PERMISSIONS);
        let _ = fs::set_permissions(path, perm);
    }

    Ok(())
}

/// Represents a temporary directory for writing cache files before atomic commit
pub struct TempCacheDir {
    /// Path to the temporary directory
    pub path: PathBuf,
    /// Whether the directory has been committed (prevents cleanup on drop)
    committed: bool,
}

impl TempCacheDir {
    /// Get paths to the cache files within this temp directory
    pub fn get_paths(&self) -> (PathBuf, PathBuf, PathBuf) {
        get_cache_paths_in_dir(&self.path)
    }
}

impl Drop for TempCacheDir {
    fn drop(&mut self) {
        if !self.committed {
            // Clean up the temp directory if we didn't commit
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Create a temporary directory for writing cache files
///
/// The temp directory is named `<digest>.tmp.<pid>.<timestamp>` to avoid collisions
/// between concurrent processes working on the same digest, even across PID reuse.
pub fn create_temp_cache_dir(cache_dir: &Path, digest: &str) -> io::Result<TempCacheDir> {
    let pid = process::id();
    let timestamp = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let temp_name = format!("{}.tmp.{}.{}", digest, pid, timestamp);
    let temp_path = cache_dir.join(temp_name);

    create_secure_dir(&temp_path)?;

    Ok(TempCacheDir {
        path: temp_path,
        committed: false,
    })
}

/// Atomically commit a temp directory to the final cache location
///
/// Uses `fs::rename` which is atomic on POSIX systems when source and dest
/// are on the same filesystem. Returns `Ok(true)` if we won the race and
/// committed successfully, or `Ok(false)` if another process already
/// committed a cache entry for this digest.
pub fn commit_cache_dir(
    temp_dir: &mut TempCacheDir,
    cache_dir: &Path,
    digest: &str,
) -> io::Result<bool> {
    let final_path = cache_dir.join(digest);

    match fs::rename(&temp_dir.path, &final_path) {
        Ok(()) => {
            temp_dir.committed = true;
            Ok(true)
        }
        Err(e)
            if e.kind() == io::ErrorKind::AlreadyExists
                || e.kind() == io::ErrorKind::DirectoryNotEmpty =>
        {
            // Another process beat us to it - that's fine, just clean up
            // (Drop will handle cleanup since committed is still false)
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

/// Clean up orphaned temporary directories in the cache
///
/// This should be called once during startup to clean up after crashes.
///
/// Strategy: delete any temp directory matching `*.tmp.*` whose modified time
/// is older than 24 hours. This avoids deleting temp dirs for currently running
/// processes while preventing unbounded growth from crashes.
pub fn cleanup_temp_dirs(cache_dir: &Path) -> io::Result<()> {
    if !cache_dir.exists() {
        return Ok(());
    }

    let cutoff = SystemTime::now().checked_sub(Duration::from_secs(60 * 60 * 24));

    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if !name.contains(".tmp.") {
            continue;
        }

        let Some(cutoff) = cutoff else {
            log::debug!("skipping temp dir {} (no cutoff)", path.display());
            continue;
        };

        let metadata = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => {
                log::debug!("skipping temp dir {} (metadata error)", path.display());
                continue;
            }
        };

        let modified = match metadata.modified() {
            Ok(m) => m,
            Err(_) => {
                log::debug!("skipping temp dir {} (modified time error)", path.display());
                continue;
            }
        };

        if modified < cutoff {
            log::debug!("cleaning up temp dir {}", path.display());
            let _ = fs::remove_dir_all(&path);
        } else {
            log::debug!("keeping temp dir {} (recent)", path.display());
        }
    }

    Ok(())
}

#[cfg(test)]
pub fn write_memo(
    cache_dir: &Path,
    digest: &str,
    memo: &Memo,
    stdout: &[u8],
    stderr: &[u8],
) -> io::Result<()> {
    let digest_dir = cache_dir.join(digest);
    fs::create_dir_all(&digest_dir)?;

    let (json_path, out_path, err_path) = get_cache_paths_in_dir(&digest_dir);

    let json = serde_json::to_string_pretty(memo)?;
    fs::write(json_path, json)?;
    fs::write(out_path, stdout)?;
    fs::write(err_path, stderr)?;

    Ok(())
}

#[cfg(test)]
pub fn read_memo(cache_dir: &Path, digest: &str) -> io::Result<(Memo, Vec<u8>, Vec<u8>)> {
    let digest_dir = cache_dir.join(digest);
    let (json_path, out_path, err_path) = get_cache_paths_in_dir(&digest_dir);

    let json = fs::read_to_string(json_path)?;
    let memo: Memo = serde_json::from_str(&json)?;
    let stdout = fs::read(out_path)?;
    let stderr = fs::read(err_path)?;

    Ok((memo, stdout, stderr))
}

/// Stream cached stdout to the given writer
pub fn stream_stdout<W: io::Write>(
    cache_dir: &Path,
    digest: &str,
    mut writer: W,
) -> io::Result<()> {
    let digest_dir = cache_dir.join(digest);
    let out_path = digest_dir.join("stdout");
    let mut file = File::open(out_path)?;
    copy(&mut file, &mut writer)?;
    Ok(())
}

/// Stream cached stderr to the given writer
pub fn stream_stderr<W: io::Write>(
    cache_dir: &Path,
    digest: &str,
    mut writer: W,
) -> io::Result<()> {
    let digest_dir = cache_dir.join(digest);
    let err_path = digest_dir.join("stderr");
    let mut file = File::open(err_path)?;
    copy(&mut file, &mut writer)?;
    Ok(())
}

/// Read just the memo metadata without loading output files
pub fn read_memo_metadata(cache_dir: &Path, digest: &str) -> io::Result<Memo> {
    let digest_dir = cache_dir.join(digest);
    let json_path = digest_dir.join("meta.json");
    let json = fs::read_to_string(json_path)?;
    let memo: Memo = serde_json::from_str(&json)?;
    Ok(memo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_cache() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let cache_dir = temp_dir.path().join("memo");
        (temp_dir, cache_dir)
    }

    #[test]
    fn test_ensure_cache_dir_creates_directory() {
        let (_temp, cache_dir) = setup_test_cache();

        assert!(!cache_dir.exists());
        ensure_cache_dir(&cache_dir).unwrap();
        assert!(cache_dir.exists());
        assert!(cache_dir.is_dir());
    }

    #[test]
    fn test_ensure_cache_dir_idempotent() {
        let (_temp, cache_dir) = setup_test_cache();

        ensure_cache_dir(&cache_dir).unwrap();
        ensure_cache_dir(&cache_dir).unwrap(); // Should not error
        assert!(cache_dir.exists());
    }

    #[test]
    fn test_write_and_read_memo() {
        let (_temp, cache_dir) = setup_test_cache();
        ensure_cache_dir(&cache_dir).unwrap();

        let digest = "abc123";
        let memo = Memo {
            cmd: vec!["echo".to_string(), "test".to_string()],
            cwd: "/test/dir".to_string(),
            exit_code: 0,
            timestamp: "2025-12-22T01:51:52.369Z".to_string(),
            digest: digest.to_string(),
        };
        let stdout = b"test output\n";
        let stderr = b"test error\n";

        write_memo(&cache_dir, digest, &memo, stdout, stderr).unwrap();

        let (read_memo, read_stdout, read_stderr) = read_memo(&cache_dir, digest).unwrap();

        assert_eq!(read_memo.cmd, memo.cmd);
        assert_eq!(read_memo.exit_code, memo.exit_code);
        assert_eq!(read_memo.digest, memo.digest);
        assert_eq!(read_stdout, stdout);
        assert_eq!(read_stderr, stderr);
    }

    #[test]
    fn test_write_memo_empty_output() {
        let (_temp, cache_dir) = setup_test_cache();
        ensure_cache_dir(&cache_dir).unwrap();

        let digest = "empty123";
        let memo = Memo {
            cmd: vec!["true".to_string()],
            cwd: "/test/dir".to_string(),
            exit_code: 0,
            timestamp: "2025-12-22T01:51:52.369Z".to_string(),
            digest: digest.to_string(),
        };

        write_memo(&cache_dir, digest, &memo, b"", b"").unwrap();

        let (_, stdout, stderr) = read_memo(&cache_dir, digest).unwrap();
        assert_eq!(stdout, b"");
        assert_eq!(stderr, b"");
    }

    #[test]
    fn test_write_memo_binary_data() {
        let (_temp, cache_dir) = setup_test_cache();
        ensure_cache_dir(&cache_dir).unwrap();

        let digest = "binary123";
        let memo = Memo {
            cmd: vec!["binary".to_string()],
            cwd: "/test/dir".to_string(),
            exit_code: 0,
            timestamp: "2025-12-22T01:51:52.369Z".to_string(),
            digest: digest.to_string(),
        };
        let binary_data = vec![0x00, 0x01, 0xFF, 0xFE, 0x7F];

        write_memo(&cache_dir, digest, &memo, &binary_data, &binary_data).unwrap();

        let (_, stdout, stderr) = read_memo(&cache_dir, digest).unwrap();
        assert_eq!(stdout, binary_data);
        assert_eq!(stderr, binary_data);
    }

    #[test]
    fn test_read_nonexistent_memo() {
        let (_temp, cache_dir) = setup_test_cache();
        ensure_cache_dir(&cache_dir).unwrap();

        let result = read_memo(&cache_dir, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_memos() {
        let (_temp, cache_dir) = setup_test_cache();
        ensure_cache_dir(&cache_dir).unwrap();

        let digest1 = "multi1";
        let digest2 = "multi2";

        let memo1 = Memo {
            cmd: vec!["echo".to_string(), "one".to_string()],
            cwd: "/test/dir".to_string(),
            exit_code: 0,
            timestamp: "2025-12-22T01:51:52.369Z".to_string(),
            digest: digest1.to_string(),
        };

        let memo2 = Memo {
            cmd: vec!["echo".to_string(), "two".to_string()],
            cwd: "/test/dir".to_string(),
            exit_code: 1,
            timestamp: "2025-12-22T01:51:52.369Z".to_string(),
            digest: digest2.to_string(),
        };

        write_memo(&cache_dir, digest1, &memo1, b"one\n", b"").unwrap();
        write_memo(&cache_dir, digest2, &memo2, b"two\n", b"err\n").unwrap();

        let (read1, out1, err1) = read_memo(&cache_dir, digest1).unwrap();
        let (read2, out2, err2) = read_memo(&cache_dir, digest2).unwrap();

        assert_eq!(read1.cmd, vec!["echo", "one"]);
        assert_eq!(read2.cmd, vec!["echo", "two"]);
        assert_eq!(out1, b"one\n");
        assert_eq!(out2, b"two\n");
        assert_eq!(err1, b"");
        assert_eq!(err2, b"err\n");
    }

    #[test]
    fn test_cache_files_have_correct_names() {
        let (_temp, cache_dir) = setup_test_cache();
        ensure_cache_dir(&cache_dir).unwrap();

        let digest = "names123";
        let memo = Memo {
            cmd: vec!["test".to_string()],
            cwd: "/test/dir".to_string(),
            exit_code: 0,
            timestamp: "2025-12-22T01:51:52.369Z".to_string(),
            digest: digest.to_string(),
        };

        write_memo(&cache_dir, digest, &memo, b"out", b"err").unwrap();

        let digest_dir = cache_dir.join(digest);
        assert!(digest_dir.join("meta.json").exists());
        assert!(digest_dir.join("stdout").exists());
        assert!(digest_dir.join("stderr").exists());
    }

    #[test]
    fn test_get_cache_dir_respects_xdg() {
        let temp = TempDir::new().unwrap();
        let xdg_path = temp.path().to_path_buf();

        std::env::set_var("XDG_CACHE_HOME", &xdg_path);
        let cache_dir = get_cache_dir().unwrap();
        std::env::remove_var("XDG_CACHE_HOME");

        assert_eq!(cache_dir, xdg_path.join("memo"));
    }

    #[test]
    fn test_large_output() {
        let (_temp, cache_dir) = setup_test_cache();
        ensure_cache_dir(&cache_dir).unwrap();

        let digest = "large123";
        let memo = Memo {
            cmd: vec!["large".to_string()],
            cwd: "/test/dir".to_string(),
            exit_code: 0,
            timestamp: "2025-12-22T01:51:52.369Z".to_string(),
            digest: digest.to_string(),
        };

        // Create 1MB of output
        let large_output = vec![b'A'; 1024 * 1024];

        write_memo(&cache_dir, digest, &memo, &large_output, b"").unwrap();

        let (_, stdout, _) = read_memo(&cache_dir, digest).unwrap();
        assert_eq!(stdout.len(), 1024 * 1024);
        assert_eq!(stdout, large_output);
    }

    #[test]
    fn test_stream_stdout() {
        let (_temp, cache_dir) = setup_test_cache();
        ensure_cache_dir(&cache_dir).unwrap();

        let digest = "stream123";
        let memo = Memo {
            cmd: vec!["test".to_string()],
            cwd: "/test/dir".to_string(),
            exit_code: 0,
            timestamp: "2025-12-22T01:51:52.369Z".to_string(),
            digest: digest.to_string(),
        };

        write_memo(&cache_dir, digest, &memo, b"output data", b"error data").unwrap();

        let mut output = Vec::new();
        stream_stdout(&cache_dir, digest, &mut output).unwrap();
        assert_eq!(output, b"output data");
    }

    #[test]
    fn test_stream_stderr() {
        let (_temp, cache_dir) = setup_test_cache();
        ensure_cache_dir(&cache_dir).unwrap();

        let digest = "stream456";
        let memo = Memo {
            cmd: vec!["test".to_string()],
            cwd: "/test/dir".to_string(),
            exit_code: 0,
            timestamp: "2025-12-22T01:51:52.369Z".to_string(),
            digest: digest.to_string(),
        };

        write_memo(&cache_dir, digest, &memo, b"output data", b"error data").unwrap();

        let mut errors = Vec::new();
        stream_stderr(&cache_dir, digest, &mut errors).unwrap();
        assert_eq!(errors, b"error data");
    }

    #[test]
    fn test_read_memo_metadata() {
        let (_temp, cache_dir) = setup_test_cache();
        ensure_cache_dir(&cache_dir).unwrap();

        let digest = "meta123";
        let memo = Memo {
            cmd: vec!["echo".to_string(), "test".to_string()],
            cwd: "/test/dir".to_string(),
            exit_code: 42,
            timestamp: "2025-12-22T01:51:52.369Z".to_string(),
            digest: digest.to_string(),
        };

        write_memo(&cache_dir, digest, &memo, b"large output here", b"errors").unwrap();

        let read_meta = read_memo_metadata(&cache_dir, digest).unwrap();
        assert_eq!(read_meta.cmd, vec!["echo", "test"]);
        assert_eq!(read_meta.exit_code, 42);
        assert_eq!(read_meta.digest, digest);
    }

    #[test]
    fn test_get_cache_paths() {
        let path = PathBuf::from("/tmp/cache");
        let (json, out, err) = get_cache_paths(&path, "abc123");

        assert_eq!(json, PathBuf::from("/tmp/cache/abc123/meta.json"));
        assert_eq!(out, PathBuf::from("/tmp/cache/abc123/stdout"));
        assert_eq!(err, PathBuf::from("/tmp/cache/abc123/stderr"));
    }
}
