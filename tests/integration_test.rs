use assert_cmd::Command;
use predicates::prelude::predicate;
use regex::Regex;
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;
use tempfile::TempDir;

/// Test environment for integration tests
///
/// Provides a clean temporary cache directory and helper methods for
/// interacting with the memo binary.
struct TestEnv {
    cache_dir: TempDir,
}

impl TestEnv {
    /// Create a new test environment with a temporary cache directory
    fn new() -> Self {
        let cache_dir = TempDir::new().unwrap();
        Self { cache_dir }
    }

    /// Get the path to the cache directory
    fn cache_path(&self) -> PathBuf {
        self.cache_dir.path().to_path_buf()
    }

    /// Create a configured Command for the memo binary
    ///
    /// The command is pre-configured with the test cache directory.
    fn cmd(&self) -> Command {
        let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("memo");
        cmd.env("XDG_CACHE_HOME", self.cache_dir.path());
        cmd
    }

    /// List all cache entries (digest directories) in sorted order
    fn list_cache_entries(&self) -> Vec<String> {
        let memo_dir = self.cache_path().join("memo");
        if !memo_dir.exists() {
            return vec![];
        }

        let mut entries: Vec<String> = fs::read_dir(&memo_dir)
            .unwrap()
            .filter_map(|e| {
                let entry = e.unwrap();
                let path = entry.path();
                // Only include directories (not temp dirs)
                if path.is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !name.contains(".tmp.") {
                        Some(name)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        entries.sort();
        entries
    }

    /// Read a cache file from within a digest directory
    fn read_cache_file(&self, digest: &str, filename: &str) -> Vec<u8> {
        let path = self.cache_path().join("memo").join(digest).join(filename);
        fs::read(&path).unwrap()
    }

    /// Assert that the cache contains exactly the specified number of entries (directories)
    fn assert_cache_entry_count(&self, expected: usize) {
        let entries = self.list_cache_entries();
        assert_eq!(
            entries.len(),
            expected,
            "Expected {} cache entries, found {}",
            expected,
            entries.len()
        );
    }

    /// Run N concurrent invocations of memo with the same command. Returns the
    /// stderr output of all invocations concatenated (for diagnosing races).
    fn run_concurrent(&self, n: usize) -> String {
        let bin = assert_cmd::cargo::cargo_bin!("memo");
        let mut children = vec![];
        for _ in 0..n {
            let child = std::process::Command::new(&bin)
                .env("XDG_CACHE_HOME", self.cache_path())
                .arg("-vv")
                .arg("bash")
                .arg("-c")
                .arg("sleep 1; echo hello")
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            children.push(child);
        }

        let mut stderr = String::new();
        for child in children {
            let output = child.wait_with_output().unwrap();
            stderr.push_str(&String::from_utf8_lossy(&output.stderr));
        }
        stderr
    }

    /// Assert that the cache contains exactly the specified number of files
    /// (backward compatibility - now counts directories)
    fn assert_cache_file_count(&self, expected: usize) {
        // With the new structure, 3 files = 1 directory
        let expected_dirs = expected / 3;
        self.assert_cache_entry_count(expected_dirs);
    }

    /// Assert that cache structure is valid (each directory has meta.json, stdout, stderr)
    fn assert_valid_cache_structure(&self) {
        let entries = self.list_cache_entries();
        let memo_dir = self.cache_path().join("memo");

        for entry in &entries {
            let digest_dir = memo_dir.join(entry);
            assert!(
                digest_dir.join("meta.json").exists(),
                "Missing meta.json in {}",
                entry
            );
            assert!(
                digest_dir.join("stdout").exists(),
                "Missing stdout in {}",
                entry
            );
            assert!(
                digest_dir.join("stderr").exists(),
                "Missing stderr in {}",
                entry
            );
        }
    }
}

// Test Case: Basic Memoization
#[test]
fn test_basic_memoization() {
    let env = TestEnv::new();

    // First run - execute
    let output1 = env
        .cmd()
        .arg("echo")
        .arg("Hello, World!")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(String::from_utf8_lossy(&output1), "Hello, World!\n");

    // Second run - replay
    let output2 = env
        .cmd()
        .arg("echo")
        .arg("Hello, World!")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(String::from_utf8_lossy(&output2), "Hello, World!\n");
    assert_eq!(output1, output2);
}

// Test Case: Verbose Mode
#[test]
fn test_verbose_mode() {
    let env = TestEnv::new();

    // First run - execute with verbose
    env.cmd()
        .arg("--verbose")
        .arg("echo")
        .arg("test")
        .assert()
        .success()
        .stdout("test\n")
        .stderr(predicate::str::contains("miss `echo test`"));

    // Second run - replay with verbose
    env.cmd()
        .arg("--verbose")
        .arg("echo")
        .arg("test")
        .assert()
        .success()
        .stdout("test\n")
        .stderr(predicate::str::contains("hit `echo test`"));
}

// Test Case: Different Commands
#[test]
fn test_different_commands() {
    let env = TestEnv::new();

    env.cmd()
        .arg("echo")
        .arg("foo")
        .assert()
        .success()
        .stdout("foo\n");

    env.cmd()
        .arg("echo")
        .arg("bar")
        .assert()
        .success()
        .stdout("bar\n");

    // Verify first command still replays correctly
    env.cmd()
        .arg("echo")
        .arg("foo")
        .assert()
        .success()
        .stdout("foo\n");
}

// Test Case: Exit Code Preservation
#[test]
fn test_exit_code_preservation() {
    let env = TestEnv::new();

    // First run - execute with non-zero exit
    env.cmd()
        .arg("sh")
        .arg("-c")
        .arg("exit 42")
        .assert()
        .code(42);

    // Second run - replay exit code
    env.cmd()
        .arg("sh")
        .arg("-c")
        .arg("exit 42")
        .assert()
        .code(42);
}

// Test Case: Stderr Capture
#[test]
fn test_stderr_capture() {
    let env = TestEnv::new();

    // First run - execute
    env.cmd()
        .arg("sh")
        .arg("-c")
        .arg("echo out; echo err >&2")
        .assert()
        .success()
        .stdout("out\n")
        .stderr("err\n");

    // Second run - replay
    env.cmd()
        .arg("sh")
        .arg("-c")
        .arg("echo out; echo err >&2")
        .assert()
        .success()
        .stdout("out\n")
        .stderr("err\n");
}

// Test Case: Argument Separator
#[test]
fn test_argument_separator() {
    let env = TestEnv::new();

    // Test -- separator prevents --verbose from being interpreted as flag
    env.cmd()
        .arg("--")
        .arg("echo")
        .arg("--verbose")
        .assert()
        .success()
        .stdout("--verbose\n");

    // Test --verbose before -- works as flag
    env.cmd()
        .arg("--verbose")
        .arg("--")
        .arg("echo")
        .arg("test")
        .assert()
        .success()
        .stdout("test\n")
        .stderr(predicate::str::contains("miss `echo test`"));
}

// Test Case: Complex Commands
#[test]
fn test_complex_commands() {
    let env = TestEnv::new();

    // First run
    let output1 = env
        .cmd()
        .arg("ls")
        .arg("-la")
        .arg("/etc/hosts")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    // Second run - should be identical
    let output2 = env
        .cmd()
        .arg("ls")
        .arg("-la")
        .arg("/etc/hosts")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(output1, output2);
}

// Test Case: Help Display
#[test]
fn test_help_display() {
    let env = TestEnv::new();

    env.cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"))
        .stdout(predicate::str::contains("--verbose"))
        .stdout(predicate::str::contains("--help"));
}

// Test Case: Version Display
#[test]
#[ignore = "seems to fail in CI"]
fn test_version_display() {
    let env = TestEnv::new();

    // Test short version format with -V
    let output = env
        .cmd()
        .arg("-V")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let short_version = String::from_utf8_lossy(&output);
    println!("Short version:\n{}", short_version);

    let short_regex = Regex::new(r#"^memo v[0-9]+\.[0-9]+\.[0-9]+.*"#).unwrap();

    assert!(
        short_regex.is_match(&short_version),
        "Short version should contain a SemVer ('memo v*.*.*'). got: {}",
        short_version
    );

    assert_eq!(
        short_version.lines().count(),
        1,
        "Short version should be one line, got: {}",
        short_version
    );

    // Test long version format with --version
    let output = env
        .cmd()
        .arg("--version")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let long_version = String::from_utf8_lossy(&output);
    println!("Long version:\n{}", long_version);

    assert!(
        long_version.contains(&*short_version),
        "Long version should contain the short version. got: {}",
        long_version
    );

    assert!(
        !long_version.contains("VERGEN"),
        "Version should not contain VERGEN placeholders, got: {}",
        long_version
    );

    assert!(
        long_version.contains("commit-id"),
        "Long version should contain 'commit-id', got: {}",
        long_version
    );
    assert!(
        long_version.contains("commit-time"),
        "Long version should contain 'commit-time', got: {}",
        long_version
    );
    assert!(
        long_version.contains("rustc-version"),
        "Long version should contain 'rustc-version', got: {}",
        long_version
    );
    assert!(
        long_version.contains("target-arch"),
        "Long version should contain 'target-arch', got: {}",
        long_version
    );
}

// Test Case: No Command Error
#[test]
fn test_no_command_error() {
    let env = TestEnv::new();

    env.cmd()
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

// Test Case: Cache Directory Creation
#[test]
fn test_cache_directory_creation() {
    let env = TestEnv::new();

    // Cache dir should not exist initially
    let memo_dir = env.cache_path().join("memo");
    assert!(!memo_dir.exists());

    // Run command
    env.cmd()
        .arg("echo")
        .arg("test")
        .assert()
        .success()
        .stdout("test\n");

    // Cache dir should now exist with three files
    assert!(memo_dir.exists());
    env.assert_cache_file_count(3);
    env.assert_valid_cache_structure();
}

// Test Case: Whitespace Handling
#[test]
fn test_whitespace_handling() {
    let env = TestEnv::new();

    // First run
    env.cmd()
        .arg("echo")
        .arg("  spaces  ")
        .assert()
        .success()
        .stdout("  spaces  \n");

    // Second run - whitespace should be preserved
    env.cmd()
        .arg("echo")
        .arg("  spaces  ")
        .assert()
        .success()
        .stdout("  spaces  \n");
}

// Test Case: Empty Output
#[test]
fn test_empty_output() {
    let env = TestEnv::new();

    // First run
    env.cmd().arg("true").assert().success().stdout("");

    // Second run
    env.cmd().arg("true").assert().success().stdout("");
}

// Additional Test: Verify cache file structure
#[test]
fn test_cache_file_structure() {
    let env = TestEnv::new();

    env.cmd().arg("echo").arg("hello").assert().success();

    let entries = env.list_cache_entries();
    assert_eq!(entries.len(), 1);

    // The entry name is the digest
    let digest = &entries[0];

    // Verify all three files exist within the digest directory
    let memo_dir = env.cache_path().join("memo").join(digest);
    assert!(memo_dir.join("meta.json").exists());
    assert!(memo_dir.join("stdout").exists());
    assert!(memo_dir.join("stderr").exists());

    // Verify stdout contains the output
    let out_content = env.read_cache_file(digest, "stdout");
    assert_eq!(out_content, b"hello\n");

    // Verify stderr is empty (echo has no stderr)
    let err_content = env.read_cache_file(digest, "stderr");
    assert_eq!(err_content, b"");

    // Verify meta.json has valid structure
    let json_content = env.read_cache_file(digest, "meta.json");
    let json: serde_json::Value = serde_json::from_slice(&json_content).unwrap();

    assert!(json["cmd"].is_array());
    assert_eq!(
        json["cmd"].as_array().unwrap(),
        &vec![
            serde_json::Value::String("echo".into()),
            serde_json::Value::String("hello".into()),
        ]
    );
    assert!(json["exit_code"].is_number());
    assert_eq!(json["exit_code"].as_i64().unwrap(), 0);
    assert!(json["env"].is_object());
    assert!(json["timestamp"].is_string());
    assert!(json["digest"].is_string());
    assert_eq!(json["digest"].as_str().unwrap(), digest);
}

// Additional Test: Binary data handling
#[test]
fn test_binary_data() {
    let env = TestEnv::new();

    // Create a command that outputs binary data
    env.cmd()
        .arg("printf")
        .arg("\\x00\\x01\\x02\\xFF")
        .assert()
        .success();

    // Replay should work with binary data
    let output = env
        .cmd()
        .arg("printf")
        .arg("\\x00\\x01\\x02\\xFF")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(output, vec![0x00, 0x01, 0x02, 0xFF]);
}

// Additional Test: Command with multiple arguments
#[test]
fn test_multiple_arguments() {
    let env = TestEnv::new();

    env.cmd()
        .arg("printf")
        .arg("%s %s %s")
        .arg("one")
        .arg("two")
        .arg("three")
        .assert()
        .success()
        .stdout("one two three");

    // Verify replay
    env.cmd()
        .arg("printf")
        .arg("%s %s %s")
        .arg("one")
        .arg("two")
        .arg("three")
        .assert()
        .success()
        .stdout("one two three");
}

// Additional Test: Command with quotes and special characters
#[test]
fn test_special_characters() {
    let env = TestEnv::new();

    env.cmd()
        .arg("echo")
        .arg("hello \"world\" $USER")
        .assert()
        .success()
        .stdout("hello \"world\" $USER\n");
}

// New test: Env vars
#[test]
fn test_env_vars() {
    let env = TestEnv::new();

    // Command 1: without FOO set
    env.cmd()
        .arg("-e")
        .arg("FOO")
        .arg("echo")
        .arg("test")
        .assert()
        .success()
        .stdout("test\n");

    // Command 2: with FOO=bar
    env.cmd()
        .env("FOO", "bar")
        .arg("-e")
        .arg("FOO")
        .arg("echo")
        .arg("test")
        .assert()
        .success()
        .stdout("test\n");

    // Should have 2 files, missing FOO and FOO=bar
    env.assert_cache_entry_count(2);

    // Command 3: run command 2 again, should be a hit (no new entry)
    env.cmd()
        .env("FOO", "bar")
        .arg("-e")
        .arg("FOO")
        .arg("echo")
        .arg("test")
        .assert()
        .success()
        .stdout("test\n");

    env.assert_cache_entry_count(2);

    // Command 4: Different working directory shouldn't create a new entry anymore
    let diff_dir = env.cache_path().join("different_dir");
    std::fs::create_dir_all(&diff_dir).unwrap();
    env.cmd()
        .current_dir(&diff_dir)
        .env("FOO", "bar")
        .arg("-e")
        .arg("FOO")
        .arg("echo")
        .arg("test")
        .assert()
        .success()
        .stdout("test\n");

    env.assert_cache_entry_count(2);
}
// Additional Test: Different commands create different cache entries
#[test]
fn test_different_cache_entries() {
    let env = TestEnv::new();

    env.cmd().arg("echo").arg("foo").assert().success();
    env.cmd().arg("echo").arg("bar").assert().success();
    env.cmd().arg("echo").arg("baz").assert().success();

    // Should have 3 directories
    env.assert_cache_entry_count(3);
    env.assert_valid_cache_structure();
}

// New test: concurrent invocations should not error with ENOTEMPTY
#[test]
fn test_concurrent_writes_no_enotempty() {
    let env = TestEnv::new();

    // Run 6 concurrent invocations
    let stderr = env.run_concurrent(6);

    // None of the stderr output should contain ENOTEMPTY/os error 39
    assert!(
        !stderr.contains("Directory not empty"),
        "Found ENOTEMPTY in stderr: {}",
        stderr
    );

    assert_eq!(1, stderr.matches("committed temp dir").count());
    assert_eq!(5, stderr.matches("dropping temp dir").count());

    // And the cache must contain exactly one entry with valid structure
    env.assert_cache_entry_count(1);
    env.assert_valid_cache_structure();
}

// Additional Test: Verify verbose flag can use short form
#[test]
fn test_verbose_short_flag() {
    let env = TestEnv::new();

    env.cmd()
        .arg("-v")
        .arg("echo")
        .arg("test")
        .assert()
        .success()
        .stdout("test\n")
        .stderr(predicate::str::contains("miss"));
}

// Additional Test: Mixed stdout/stderr with exit code
#[test]
fn test_quiet_mode() {
    let env = TestEnv::new();

    // With --quiet, a missing command should exit with 1 but NOT print a :: memo :: ERROR
    env.cmd()
        .arg("--quiet")
        .arg("this_command_does_not_exist_xyz123")
        .assert()
        .failure()
        .stderr(predicate::str::is_empty());
}

#[test]
fn test_mixed_output_with_error() {
    let env = TestEnv::new();

    env.cmd()
        .arg("sh")
        .arg("-c")
        .arg("echo stdout; echo stderr >&2; exit 5")
        .assert()
        .code(5)
        .stdout("stdout\n")
        .stderr("stderr\n");

    // Verify replay preserves all three
    env.cmd()
        .arg("sh")
        .arg("-c")
        .arg("echo stdout; echo stderr >&2; exit 5")
        .assert()
        .code(5)
        .stdout("stdout\n")
        .stderr("stderr\n");
}

// Test Case: Argv Collision Avoidance
#[test]
fn test_argv_collision_avoidance() {
    let env = TestEnv::new();

    // Command 1: echo "a b" (one argument after echo)
    env.cmd()
        .arg("-v")
        .arg("echo")
        .arg("a b")
        .assert()
        .success()
        .stdout("a b\n")
        .stderr(predicate::str::contains("miss `echo a b`"));

    // Command 2: echo a b (two arguments after echo)
    // If these collided, this would be a cache hit.
    env.cmd()
        .arg("-v")
        .arg("echo")
        .arg("a")
        .arg("b")
        .assert()
        .success()
        .stdout("a b\n")
        .stderr(predicate::str::contains("miss `echo a b`"));

    // Verify we have two distinct cache entries
    env.assert_cache_entry_count(2);
}
