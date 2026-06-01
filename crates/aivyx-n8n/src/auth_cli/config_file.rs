//! Config file loading for `aivyx-n8n`.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::N8nConfig;

#[derive(Debug, Error)]
pub enum ConfigFileError {
    #[error("config file at {path:?} not found — create it with `n8n_base_url = \"...\"` and `n8n_api_key = \"...\"`. See `aivyx-n8n help`.")]
    NotFound { path: PathBuf },
    #[error("I/O error reading {path:?}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("parse error in {path:?}: {reason}")]
    Parse { path: PathBuf, reason: String },
    #[error("config field `n8n_base_url` is empty in {path:?}")]
    EmptyBaseUrl { path: PathBuf },
    #[error("config field `n8n_api_key` is empty in {path:?}")]
    EmptyApiKey { path: PathBuf },
}

pub fn default_config_path() -> Result<PathBuf, ConfigFileError> {
    crate::default_config_path().ok_or_else(|| ConfigFileError::NotFound {
        path: PathBuf::from("~/.aivyx/tool-processes/n8n/config.toml"),
    })
}

pub fn load_config(path: &Path) -> Result<N8nConfig, ConfigFileError> {
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
    let cfg: N8nConfig = toml::from_str(&body).map_err(|e| ConfigFileError::Parse {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    if cfg.n8n_base_url.trim().is_empty() {
        return Err(ConfigFileError::EmptyBaseUrl {
            path: path.to_path_buf(),
        });
    }
    if cfg.n8n_api_key.trim().is_empty() {
        return Err(ConfigFileError::EmptyApiKey {
            path: path.to_path_buf(),
        });
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpfile(body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "aivyx-n8n-cfg-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    #[test]
    fn returns_not_found_when_missing() {
        let e = load_config(Path::new("/nope/missing.toml")).expect_err("must error");
        assert!(matches!(e, ConfigFileError::NotFound { .. }));
    }

    #[test]
    fn returns_parse_for_malformed_toml() {
        let path = tmpfile("not valid ===");
        let e = load_config(&path).expect_err("must error");
        assert!(matches!(e, ConfigFileError::Parse { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_empty_base_url() {
        let path = tmpfile(
            r#"n8n_base_url = "  "
n8n_api_key = "k""#,
        );
        let e = load_config(&path).expect_err("must error");
        assert!(matches!(e, ConfigFileError::EmptyBaseUrl { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rejects_empty_api_key() {
        let path = tmpfile(
            r#"n8n_base_url = "https://n8n.example.com"
n8n_api_key = """#,
        );
        let e = load_config(&path).expect_err("must error");
        assert!(matches!(e, ConfigFileError::EmptyApiKey { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn accepts_valid_config() {
        let path = tmpfile(
            r#"n8n_base_url = "https://n8n.example.com"
n8n_api_key = "ntn_test""#,
        );
        let cfg = load_config(&path).expect("ok");
        assert_eq!(cfg.n8n_base_url, "https://n8n.example.com");
        assert_eq!(cfg.n8n_api_key, "ntn_test");
        let _ = std::fs::remove_file(&path);
    }
}
