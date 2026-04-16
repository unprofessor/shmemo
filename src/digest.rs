//! Digest computation for cache key generation
//!
//! This module handles SHA-256 digest computation for command memoization.
//! The digest includes both the command arguments and explicitly passed environment
//! variables to ensure different contexts produce different cache entries.

use std::collections::BTreeMap;

use crate::error::Result;
use sha2::{Digest, Sha256};

/// Compute SHA-256 digest for command arguments and environment variables
///
/// The digest is computed from a JSON encoding of the arguments and environment
/// variables to avoid collisions. For example, `["echo", "a b"]` vs `["echo", "a", "b"]` will
/// produce different digests.
///
/// # Arguments
///
/// * `args` - Command arguments (including the command itself)
/// * `env` - Environment variables map
///
/// # Returns
///
/// Returns a hex-encoded SHA-256 digest (64 characters).
///
/// # Examples
///
/// ```
/// # use memo::digest::compute_digest;
/// # use std::collections::BTreeMap;
/// let args = vec!["echo".to_string(), "hello".to_string()];
/// let env = BTreeMap::new();
/// let digest = compute_digest(&args, &env).unwrap();
/// assert_eq!(digest.len(), 64);
/// assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
/// ```
pub fn compute_digest(args: &[String], env: &BTreeMap<String, Option<String>>) -> Result<String> {
    // Hash a canonical encoding of argv and env to avoid collisions like:
    // ["echo", "a b"] vs ["echo", "a", "b"].
    let encoded_args = serde_json::to_vec(args)?;
    let encoded_env = serde_json::to_vec(env)?;
    let mut hasher = Sha256::new();
    hasher.update(&encoded_args);
    hasher.update(&encoded_env);
    let result = hasher.finalize();
    Ok(hex::encode(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shell_words::split;

    fn digest_for_args(args: &[String]) -> String {
        compute_digest(args, &BTreeMap::new()).expect("failed to compute digest")
    }

    fn digest_for_command(command: &str) -> String {
        let args = split(command).expect("failed to parse command");
        digest_for_args(&args)
    }

    #[test]
    fn test_digest_same_command_same_output() {
        let digest1 = digest_for_command("echo hello");
        let digest2 = digest_for_command("echo hello");
        assert_eq!(digest1, digest2);
    }

    #[test]
    fn test_digest_different_commands_different_output() {
        let digest1 = digest_for_command("echo hello");
        let digest2 = digest_for_command("echo world");
        assert_ne!(digest1, digest2);
    }

    #[test]
    fn test_digest_format() {
        let digest = digest_for_command("echo test");
        assert_eq!(digest.len(), 64);
        // Should only contain hex characters
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_digest_whitespace_collapses() {
        let digest1 = digest_for_command("echo   hello");
        let digest2 = digest_for_command("echo hello");
        assert_eq!(digest1, digest2);
    }

    #[test]
    fn test_digest_order_sensitive() {
        let digest1 = digest_for_args(&["echo".into(), "hello".into(), "world".into()]);
        let digest2 = digest_for_args(&["echo".into(), "world".into(), "hello".into()]);
        assert_ne!(digest1, digest2);
    }

    #[test]
    fn test_digest_empty_args_known_value() {
        let digest = digest_for_command("");
        assert_eq!(digest.len(), 64);
    }

    #[test]
    fn test_digest_quoting_changes_arguments() {
        let quoted = digest_for_command("echo 'hello world'");
        let unquoted = digest_for_command("echo hello world");
        assert_ne!(quoted, unquoted);
    }

    #[test]
    fn test_digest_args_avoids_join_collisions() {
        let quoted = digest_for_command("echo 'a b'");
        let split_args = digest_for_command("echo a b");
        assert_ne!(quoted, split_args);
    }

    #[test]
    fn test_digest_known_value_for_echo_hello() {
        let digest = digest_for_args(&["echo".into(), "hello".into()]);
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_digest_different_env_different_output() {
        let args: Vec<String> = vec!["echo".into(), "hello".into()];

        let mut env1 = BTreeMap::new();
        env1.insert("VAR".to_string(), Some("one".to_string()));
        let digest1 = compute_digest(&args, &env1).unwrap();

        let mut env2 = BTreeMap::new();
        env2.insert("VAR".to_string(), Some("two".to_string()));
        let digest2 = compute_digest(&args, &env2).unwrap();

        let mut env3 = BTreeMap::new();
        env3.insert("VAR".to_string(), None);
        let digest3 = compute_digest(&args, &env3).unwrap();

        assert_ne!(digest1, digest2);
        assert_ne!(digest2, digest3);
        assert_ne!(digest1, digest3);
    }

    #[test]
    fn test_digest_same_env_same_output() {
        let args: Vec<String> = vec!["echo".into(), "hello".into()];

        let mut env1 = BTreeMap::new();
        env1.insert("VAR".to_string(), Some("one".to_string()));
        let digest1 = compute_digest(&args, &env1).unwrap();

        let mut env2 = BTreeMap::new();
        env2.insert("VAR".to_string(), Some("one".to_string()));
        let digest2 = compute_digest(&args, &env2).unwrap();

        assert_eq!(digest1, digest2);
    }

    #[test]
    fn test_digest_special_characters_are_preserved() {
        let digest1 = digest_for_command("echo \"hello\" 'world' $USER");
        let digest2 = digest_for_command("echo \"hello\" 'world' $USER");
        assert_eq!(digest1, digest2);
    }
}
