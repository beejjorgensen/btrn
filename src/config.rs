use std::path::PathBuf;
use std::time::Duration;

pub struct Config {
    pub server: Option<String>,
    pub port: u16,
    pub use_tls: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    pub password_command: Option<Vec<String>>,
    pub editor: String,
    pub config_dir: PathBuf,
    pub newsrc_path: PathBuf,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub initial_overview_limit: usize,
    pub overview_chunk_size: usize,
}

impl Config {
    pub fn from_environment() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let editor = ["BTRN_EDITOR", "EDITOR"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|value| value.to_string_lossy().into_owned())
            .find(|value| !value.is_empty())
            .unwrap_or_else(|| "vim".to_owned());
        Self::with_paths(home.join(".config/btrn"), editor)
    }

    pub fn with_paths(config_dir: PathBuf, editor: String) -> Self {
        let newsrc_path = config_dir.join("newsrc");
        Self {
            server: None,
            port: 563,
            use_tls: true,
            username: None,
            password: None,
            password_command: None,
            editor,
            config_dir,
            newsrc_path,
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            initial_overview_limit: 10_000,
            overview_chunk_size: 1_000,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::from_environment()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_use_secure_bounded_nntp_and_overview_settings() {
        let config = Config::with_paths(PathBuf::from("/tmp/btrn-config"), "vim".into());
        assert!(config.use_tls);
        assert_eq!(config.port, 563);
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
        assert_eq!(config.read_timeout, Duration::from_secs(30));
        assert_eq!(config.initial_overview_limit, 10_000);
        assert_eq!(config.overview_chunk_size, 1_000);
        assert_eq!(config.newsrc_path, PathBuf::from("/tmp/btrn-config/newsrc"));
    }
}
