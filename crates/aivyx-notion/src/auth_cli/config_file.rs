//! Config file loading for `aivyx-notion`.
//!
//! Phase 130 Task 2. Reads the operator-supplied
//! `~/.aivyx/tool-processes/notion/config.toml` carrying
//! the Notion Integration Token.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::NotionConfig;

#[derive(Debug, Error)]
pub enum ConfigFileError {
    #[error("config file at {path:?} not found — create it with the operator's Notion Integration Token. Format: `notion_token = \"ntn_...\"`. See `aivyx-notion help` for full setup.")]
    NotFound { path: PathBuf },
    #[error("I/O error reading {path:?}: {source}")]
    Io {
        path: PathBuf,
        source: io::Error,
    },
    #[error("parse error in {path:?}: {reason}")]
    Parse { path: PathBuf, reason: String },
    #[error("config field `notion_token` is empty in {path:?}")]
    EmptyToken { path: PathBuf },
}

pub fn default_config_path() -> Result<PathBuf, ConfigFileError> {
    crate::default_config_path().ok_or_else(|| ConfigFileError::NotFound {
        path: PathBuf::from("~/.aivyx/tool-processes/notion/config.toml"),
    })
}

pub fn load_config(path: &Path) -> Result<NotionConfig, ConfigFileError> {
    let body = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(ConfigFileError::NotFound {
                path: path.to_path_buf(),
            });
        }
        Err(e) => {
            return Err(ConfigFileError::Io {
                path: path.to_path_buf(),
                source: e,
            });
        }
    };
    let cfg: NotionConfig =
        toml::from_str(&body).map_err(|e| ConfigFileError::Parse {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
    if cfg.notion_token.trim().is_empty() {
        return Err(ConfigFileError::EmptyToken {
            path: path.to_path_buf(),
        });
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpfile(contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "aivyx-notion-cfg-{}-{}.toml",
            std::process::id(),
            uuid_like_suffix()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    fn uuid_like_suffix() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }

    #[test]
    fn load_config_returns_not_found_for_missing_path() {
        let nonexistent = std::env::temp_dir().join(format!(
            "aivyx-notion-missing-{}.toml",
            uuid_like_suffix()
        ));
        let e = load_config(&nonexistent).expect_err("must error");
        assert!(matches!(e, ConfigFileError::NotFound { .. }));
    }

    #[test]
    fn load_config_returns_parse_for_malformed_toml() {
        let path = tmpfile("not valid toml ====");
        let e = load_config(&path).expect_err("must error");
        assert!(matches!(e, ConfigFileError::Parse { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_config_rejects_empty_token() {
        let path = tmpfile(r#"notion_token = "  ""#);
        let e = load_config(&path).expect_err("must error");
        assert!(matches!(e, ConfigFileError::EmptyToken { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_config_accepts_valid_token() {
        let path = tmpfile(r#"notion_token = "ntn_test_xyz""#);
        let cfg = load_config(&path).expect("load");
        assert_eq!(cfg.notion_token, "ntn_test_xyz");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn not_found_error_message_mentions_setup_instructions() {
        let path = PathBuf::from("/nonexistent/path.toml");
        let e = ConfigFileError::NotFound { path };
        let s = e.to_string();
        assert!(s.contains("Integration Token"));
        assert!(s.contains("notion_token"));
        assert!(s.contains("aivyx-notion help"));
    }
}
