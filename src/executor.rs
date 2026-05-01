//! Command execution with output streaming
//!
//! This module handles the execution of shell commands and streaming their output
//! directly to cache files and console simultaneously. This avoids loading large
//! outputs into memory while providing real-time console feedback.

use crate::constants::FILE_PERMISSIONS;
use crate::error::{Result, ShmemoError};
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// Result of command execution
pub struct ExecutionResult {
    /// The exit code returned by the command
    pub exit_code: i32,
    /// Error encountered while writing to stdout file (if any)
    pub stdout_error: Option<PathBuf>,
    /// Error encountered while writing to stderr file (if any)
    pub stderr_error: Option<PathBuf>,
}

/// A writer that duplicates writes to two destinations
///
/// TeeWriter writes to both a file and the console simultaneously, allowing
/// real-time output while caching. If file writes fail, it continues with
/// console output and stores the error for later reporting.
struct TeeWriter<W: Write> {
    file: File,
    console: W,
    file_path: PathBuf,
    error: RefCell<Option<io::Error>>,
}

impl<W: Write> TeeWriter<W> {
    fn new(file: File, console: W, file_path: PathBuf) -> Self {
        Self {
            file,
            console,
            file_path,
            error: RefCell::new(None),
        }
    }

    fn has_error(&self) -> bool {
        self.error.borrow().is_some()
    }

    fn take_error_path(&self) -> Option<PathBuf> {
        if self.has_error() {
            Some(self.file_path.clone())
        } else {
            None
        }
    }
}

impl<W: Write> Write for TeeWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Try to write to file first
        let file_result = self.file.write_all(buf);

        // Always write to console
        let console_result = self.console.write_all(buf);

        // Store file error if it occurred
        if let Err(e) = file_result {
            if self.error.borrow().is_none() {
                *self.error.borrow_mut() = Some(e);
            }
        }

        // Return console result (file errors are stored, not returned)
        console_result?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // Try to flush file, but don't fail on error
        let _ = self.file.flush();

        // Always flush console
        self.console.flush()
    }
}

/// Build a display string from command arguments
///
/// Joins arguments with spaces for user-friendly display.
///
/// # Examples
///
/// ```
/// # use shmemo::executor::build_command_string;
/// let args = vec!["echo".to_string(), "hello".to_string()];
/// assert_eq!(build_command_string(&args), "echo hello");
/// ```
pub fn build_command_string(args: &[String]) -> String {
    args.join(" ")
}

/// Create a new file with secure permissions (owner read/write only)
fn create_secure_file(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);

    #[cfg(unix)]
    {
        opts.mode(FILE_PERMISSIONS);
    }

    opts.open(path)
}

/// Execute a command and stream its output directly to files and console
///
/// This function creates the output files with secure permissions and streams
/// stdout and stderr simultaneously to both files and console in real-time.
/// If file writes fail, the command continues with console-only output and
/// errors are reported in the result.
///
/// # Arguments
///
/// * `args` - Command and its arguments (first element is the command)
/// * `stdout_path` - Path where stdout will be written
/// * `stderr_path` - Path where stderr will be written
///
/// # Returns
///
/// Returns an `ExecutionResult` containing the exit code and any file write errors.
///
/// # Errors
///
/// Returns an error if:
/// - No command is provided
/// - File creation fails
/// - Command execution fails
///
/// # Examples
///
/// ```no_run
/// # use shmemo::executor::execute_and_stream;
/// # use std::path::Path;
/// let result = execute_and_stream(
///     &["echo", "hello"],
///     Path::new("/tmp/out.txt"),
///     Path::new("/tmp/err.txt")
/// ).expect("Command failed");
/// assert_eq!(result.exit_code, 0);
/// ```
pub fn execute_and_stream(
    args: &[&str],
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<ExecutionResult> {
    if args.is_empty() {
        return Err(ShmemoError::InvalidCommand(
            "No command provided".to_string(),
        ));
    }

    let stdout_file = create_secure_file(stdout_path)?;
    let stderr_file = create_secure_file(stderr_path)?;

    // Create TeeWriters that write to both file and console
    let mut stdout_tee = TeeWriter::new(stdout_file, io::stdout(), stdout_path.to_path_buf());
    let mut stderr_tee = TeeWriter::new(stderr_file, io::stderr(), stderr_path.to_path_buf());

    // Spawn the command with piped stdout/stderr
    let mut child = Command::new(args[0])
        .args(&args[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Take the stdout and stderr handles
    let mut child_stdout = child.stdout.take().expect("Failed to capture stdout");
    let mut child_stderr = child.stderr.take().expect("Failed to capture stderr");

    // Copy from child's stdout/stderr to our TeeWriters
    // We ignore copy errors since TeeWriter handles them internally
    let _ = io::copy(&mut child_stdout, &mut stdout_tee);
    let _ = io::copy(&mut child_stderr, &mut stderr_tee);

    // Wait for the command to complete
    let status = child.wait()?;
    let exit_code = status.code().unwrap_or(-1);

    // Collect any file write errors
    let stdout_error = stdout_tee.take_error_path();
    let stderr_error = stderr_tee.take_error_path();

    Ok(ExecutionResult {
        exit_code,
        stdout_error,
        stderr_error,
    })
}

/// Execute a command and stream output directly to stdout/stderr
///
/// This function executes a command without any caching, streaming output
/// directly to the current process's stdout and stderr.
///
/// # Arguments
///
/// * `args` - Command and its arguments (first element is the command)
///
/// # Returns
///
/// Returns an `ExecutionResult` containing the exit code.
///
/// # Errors
///
/// Returns an error if:
/// - No command is provided
/// - Command execution fails
///
/// # Examples
///
/// ```no_run
/// # use shmemo::executor::execute_direct;
/// let result = execute_direct(&["echo", "hello"]).expect("Command failed");
/// assert_eq!(result.exit_code, 0);
/// ```
pub fn execute_direct(args: &[&str]) -> Result<ExecutionResult> {
    if args.is_empty() {
        return Err(ShmemoError::InvalidCommand(
            "No command provided".to_string(),
        ));
    }

    let status = Command::new(args[0]).args(&args[1..]).status()?;

    let exit_code = status.code().unwrap_or(-1);

    Ok(ExecutionResult {
        exit_code,
        stdout_error: None,
        stderr_error: None,
    })
}

/// Execute command for testing (keeps output in memory)
#[cfg(test)]
fn execute_command(args: &[&str]) -> std::io::Result<TestExecutionResult> {
    if args.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "No command provided",
        ));
    }

    let output = Command::new(args[0])
        .args(&args[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()?;

    let exit_code = output.status.code().unwrap_or(-1);

    Ok(TestExecutionResult {
        stdout: output.stdout,
        stderr: output.stderr,
        exit_code,
    })
}

#[cfg(test)]
struct TestExecutionResult {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_execute_simple_command() {
        let result = execute_command(&["echo", "hello"]).unwrap();
        assert_eq!(result.stdout, b"hello\n");
        assert_eq!(result.stderr, b"");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_with_stderr() {
        let result = execute_command(&["sh", "-c", "echo error >&2"]).unwrap();
        assert_eq!(result.stdout, b"");
        assert_eq!(result.stderr, b"error\n");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_with_exit_code() {
        let result = execute_command(&["sh", "-c", "exit 42"]).unwrap();
        assert_eq!(result.exit_code, 42);
    }

    #[test]
    fn test_execute_mixed_output() {
        let result = execute_command(&["sh", "-c", "echo out; echo err >&2"]).unwrap();
        assert_eq!(result.stdout, b"out\n");
        assert_eq!(result.stderr, b"err\n");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_multiple_args() {
        let result = execute_command(&["printf", "%s %s", "hello", "world"]).unwrap();
        assert_eq!(result.stdout, b"hello world");
    }

    #[test]
    fn test_execute_empty_output() {
        let result = execute_command(&["true"]).unwrap();
        assert_eq!(result.stdout, b"");
        assert_eq!(result.stderr, b"");
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_failure() {
        let result = execute_command(&["false"]).unwrap();
        assert_eq!(result.exit_code, 1);
    }

    #[test]
    fn test_execute_with_special_chars() {
        let result = execute_command(&["echo", "hello \"world\""]).unwrap();
        assert_eq!(result.stdout, b"hello \"world\"\n");
    }

    #[test]
    fn test_execute_invalid_command() {
        let result = execute_command(&["this-command-does-not-exist-xyz"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_with_env_vars() {
        // Commands should execute in current environment
        let result = execute_command(&["sh", "-c", "echo $HOME"]).unwrap();
        // Should have some output (the HOME value)
        assert!(!result.stdout.is_empty());
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_execute_binary_output() {
        let result = execute_command(&["printf", "\\x00\\x01\\xFF"]).unwrap();
        assert_eq!(result.stdout, vec![0x00, 0x01, 0xFF]);
    }

    #[test]
    fn test_execute_and_stream() {
        let temp_dir = TempDir::new().unwrap();
        let stdout_path = temp_dir.path().join("out");
        let stderr_path = temp_dir.path().join("err");

        let result = execute_and_stream(
            &["sh", "-c", "echo hello; echo world >&2"],
            &stdout_path,
            &stderr_path,
        )
        .unwrap();

        assert_eq!(result.exit_code, 0);
        assert_eq!(fs::read(&stdout_path).unwrap(), b"hello\n");
        assert_eq!(fs::read(&stderr_path).unwrap(), b"world\n");
    }

    #[test]
    fn test_execute_and_stream_binary() {
        let temp_dir = TempDir::new().unwrap();
        let stdout_path = temp_dir.path().join("out");
        let stderr_path = temp_dir.path().join("err");

        let result =
            execute_and_stream(&["printf", "\\x00\\x01\\xFF"], &stdout_path, &stderr_path).unwrap();

        assert_eq!(result.exit_code, 0);
        assert_eq!(fs::read(&stdout_path).unwrap(), vec![0x00, 0x01, 0xFF]);
    }

    #[test]
    fn test_build_command_string() {
        let cmd =
            build_command_string(&["echo".to_string(), "hello".to_string(), "world".to_string()]);
        assert_eq!(cmd, "echo hello world");
    }

    #[test]
    fn test_build_command_string_single_arg() {
        let cmd = build_command_string(&["ls".to_string()]);
        assert_eq!(cmd, "ls");
    }

    #[test]
    fn test_build_command_string_empty() {
        let cmd = build_command_string(&[]);
        assert_eq!(cmd, "");
    }

    #[test]
    fn test_build_command_string_with_spaces() {
        let cmd = build_command_string(&["echo".to_string(), "hello world".to_string()]);
        assert_eq!(cmd, "echo hello world");
    }

    #[test]
    fn test_build_command_string_special_chars() {
        let cmd = build_command_string(&[
            "echo".to_string(),
            "\"quoted\"".to_string(),
            "$VAR".to_string(),
        ]);
        assert_eq!(cmd, "echo \"quoted\" $VAR");
    }
}
