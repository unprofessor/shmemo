//! Memo metadata structure
//!
//! This module defines the metadata structure that is serialized to JSON
//! and stored in the cache directory.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Metadata for a memoized command execution
///
/// This structure is serialized to JSON and stored in `<digest>.json`.
/// It does not contain the actual stdout/stderr data, which are stored
/// separately in `.out` and `.err` files.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Memo {
    /// The command arguments that were executed
    pub cmd: Vec<String>,
    /// Environment variables considered for the cache key
    pub env: BTreeMap<String, Option<String>>,
    /// The exit code returned by the command
    pub exit_code: i32,
    /// ISO 8601 timestamp of when the command was executed
    pub timestamp: String,
    /// ISO 8601 timestamp of when the cache entry expires, if TTL was specified
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// SHA-256 digest used as the cache key
    pub digest: String,
}

impl Memo {
    /// Check if the cache entry has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at_str) = &self.expires_at {
            if let Ok(expires_at) = chrono::DateTime::parse_from_rfc3339(expires_at_str) {
                return chrono::Utc::now() > expires_at.with_timezone(&chrono::Utc);
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ts() -> String {
        "2025-12-22T01:51:52.369Z".to_string()
    }

    #[test]
    fn test_memo_serialization() {
        let mut env = BTreeMap::new();
        env.insert("FOO".to_string(), Some("bar".to_string()));
        env.insert("MISSING".to_string(), None);

        let memo = Memo {
            cmd: vec!["echo".to_string(), "hello".to_string()],
            env,
            exit_code: 0,
            timestamp: ts(),
            digest: "abc123".to_string(),
            expires_at: None,
        };

        let json_str = serde_json::to_string(&memo).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        assert_eq!(value["cmd"], json!(["echo", "hello"]));
        assert_eq!(
            value["env"],
            json!({
                "FOO": "bar",
                "MISSING": null
            })
        );
        assert_eq!(value["exit_code"], json!(0));
        assert_eq!(value["digest"], json!("abc123"));
    }

    #[test]
    fn test_memo_deserialization() {
        let json_str = r#"{
            "cmd": ["echo", "test"],
            "env": {
                "SOME_VAR": "value",
                "UNSET_VAR": null
            },
            "exit_code": 42,
            "timestamp": "2025-12-22T01:51:52.369Z",
            "digest": "def456"
        }"#;

        let memo: Memo = serde_json::from_str(json_str).unwrap();
        assert_eq!(memo.cmd, vec!["echo", "test"]);
        assert_eq!(memo.env.get("SOME_VAR").unwrap().as_deref(), Some("value"));
        assert_eq!(memo.env.get("UNSET_VAR").unwrap(), &None);
        assert_eq!(memo.exit_code, 42);
        assert_eq!(memo.digest, "def456");
    }

    #[test]
    fn test_memo_roundtrip() {
        let original = Memo {
            cmd: vec!["ls".to_string(), "-la".to_string()],
            env: BTreeMap::new(),
            exit_code: 1,
            timestamp: ts(),
            digest: "xyz789".to_string(),
            expires_at: None,
        };

        let json_str = serde_json::to_string(&original).unwrap();
        let deserialized: Memo = serde_json::from_str(&json_str).unwrap();

        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_memo_with_special_characters() {
        let memo = Memo {
            cmd: vec!["echo".to_string(), "\"hello\" 'world' $USER".to_string()],
            env: BTreeMap::new(),
            exit_code: 0,
            timestamp: ts(),
            digest: "special123".to_string(),
            expires_at: None,
        };

        let json_str = serde_json::to_string(&memo).unwrap();
        let deserialized: Memo = serde_json::from_str(&json_str).unwrap();

        assert_eq!(memo.cmd, deserialized.cmd);
    }

    #[test]
    fn test_memo_negative_exit_code() {
        let memo = Memo {
            cmd: vec!["test".to_string()],
            env: BTreeMap::new(),
            exit_code: -1,
            timestamp: ts(),
            digest: "neg123".to_string(),
            expires_at: None,
        };

        let json_str = serde_json::to_string(&memo).unwrap();
        let deserialized: Memo = serde_json::from_str(&json_str).unwrap();

        assert_eq!(memo.exit_code, deserialized.exit_code);
    }

    #[test]
    fn test_memo_multiline_command() {
        let memo = Memo {
            cmd: vec![
                "sh".to_string(),
                "-c".to_string(),
                "echo hello\necho world".to_string(),
            ],
            env: BTreeMap::new(),
            exit_code: 0,
            timestamp: ts(),
            digest: "multi123".to_string(),
            expires_at: None,
        };

        let json_str = serde_json::to_string(&memo).unwrap();
        let deserialized: Memo = serde_json::from_str(&json_str).unwrap();

        assert_eq!(memo.cmd, deserialized.cmd);
    }
}
