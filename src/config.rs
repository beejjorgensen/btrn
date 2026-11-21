use serde::Deserialize;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::process::Command;
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

pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    InvalidLine { line_number: Option<usize> },
    UnknownKey { line_number: Option<usize> },
    InvalidValue { line_number: Option<usize> },
    MissingServer,
    MissingUsername,
    MissingPassword,
    ConflictingPasswordSources,
    EmptyPassword,
    PasswordCommandStart,
    PasswordCommandFailed,
    PasswordNotUtf8,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "could not read config: {error}"),
            Self::InvalidLine { line_number } => {
                write!(formatter, "invalid config syntax{}", on_line(*line_number))
            }
            Self::UnknownKey { line_number } => {
                write!(formatter, "unknown config key{}", on_line(*line_number))
            }
            Self::InvalidValue { line_number } => {
                write!(formatter, "invalid config value{}", on_line(*line_number))
            }
            Self::MissingServer => write!(
                formatter,
                "server is required (set it in config or on the command line)"
            ),
            Self::MissingUsername => write!(
                formatter,
                "username is required when a password is configured"
            ),
            Self::MissingPassword => write!(
                formatter,
                "password or password_command is required for authentication"
            ),
            Self::ConflictingPasswordSources => write!(
                formatter,
                "configure either password or password_command, not both"
            ),
            Self::EmptyPassword => write!(formatter, "resolved password is empty"),
            Self::PasswordCommandStart => write!(formatter, "could not start password command"),
            Self::PasswordCommandFailed => write!(formatter, "password command failed"),
            Self::PasswordNotUtf8 => {
                write!(formatter, "password command output is not valid UTF-8")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

fn on_line(line_number: Option<usize>) -> String {
    line_number.map_or_else(String::new, |line| format!(" on line {line}"))
}

/// Top-level keys accepted in `config.toml`; must match `ConfigFile`.
const KNOWN_KEYS: [&str; 11] = [
    "server",
    "port",
    "use_tls",
    "username",
    "password",
    "password_command",
    "editor",
    "connect_timeout",
    "read_timeout",
    "initial_overview_limit",
    "overview_chunk_size",
];

/// The shape of `config.toml`. Unset keys leave `Config` defaults in place.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    server: Option<String>,
    port: Option<NonZeroU16>,
    use_tls: Option<bool>,
    username: Option<String>,
    password: Option<String>,
    password_command: Option<Vec<String>>,
    editor: Option<String>,
    connect_timeout: Option<u64>,
    read_timeout: Option<u64>,
    initial_overview_limit: Option<usize>,
    overview_chunk_size: Option<usize>,
}

/// Classifies a `toml` error by category and line only. The crate's own
/// messages can quote config values such as passwords, so they are never used.
fn config_error(input: &str, error: &toml::de::Error) -> ConfigError {
    let line_number = error.span().map(|span| {
        input[..span.start.min(input.len())]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1
    });
    match input.parse::<toml::Table>() {
        Err(_) => ConfigError::InvalidLine { line_number },
        Ok(table) if table.keys().any(|key| !KNOWN_KEYS.contains(&key.as_str())) => {
            ConfigError::UnknownKey { line_number }
        }
        Ok(_) => ConfigError::InvalidValue { line_number },
    }
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

    pub fn load() -> Result<Self, ConfigError> {
        let defaults = Self::from_environment();
        let mut config = Self::load_from_dir(defaults.config_dir.clone(), defaults.editor.clone())?;
        if let Some(editor) = ["BTRN_EDITOR", "EDITOR"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|value| value.to_string_lossy().into_owned())
            .find(|value| !value.is_empty())
        {
            config.editor = editor;
        }
        Ok(config)
    }

    pub fn load_from_dir(config_dir: PathBuf, editor: String) -> Result<Self, ConfigError> {
        let mut config = Self::with_paths(config_dir, editor);
        let path = config.config_dir.join("config.toml");
        match std::fs::read_to_string(path) {
            Ok(contents) => config.apply_toml(&contents)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(ConfigError::Io(error)),
        }
        Ok(config)
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

    pub fn apply_toml(&mut self, input: &str) -> Result<(), ConfigError> {
        let file: ConfigFile =
            toml::from_str(input).map_err(|error| config_error(input, &error))?;
        if let Some(server) = file.server {
            self.server = Some(server);
        }
        if let Some(port) = file.port {
            self.port = port.get();
        }
        if let Some(use_tls) = file.use_tls {
            self.use_tls = use_tls;
        }
        if let Some(username) = file.username {
            self.username = Some(username);
        }
        if let Some(password) = file.password {
            self.password = Some(password);
        }
        if let Some(command) = file.password_command {
            self.password_command = Some(command);
        }
        if let Some(editor) = file.editor {
            self.editor = editor;
        }
        if let Some(seconds) = file.connect_timeout {
            self.connect_timeout = Duration::from_secs(seconds);
        }
        if let Some(seconds) = file.read_timeout {
            self.read_timeout = Duration::from_secs(seconds);
        }
        if let Some(limit) = file.initial_overview_limit {
            self.initial_overview_limit = limit;
        }
        if let Some(size) = file.overview_chunk_size {
            self.overview_chunk_size = size;
        }
        Ok(())
    }

    pub fn resolve_credentials(&self) -> Result<Option<Credentials>, ConfigError> {
        let has_password_source = self.password.is_some() || self.password_command.is_some();
        let Some(username) = &self.username else {
            return if has_password_source {
                Err(ConfigError::MissingUsername)
            } else {
                Ok(None)
            };
        };
        let password = match (&self.password, &self.password_command) {
            (Some(password), None) => password.clone(),
            (None, Some(command)) => resolve_password_command(command)?,
            (Some(_), Some(_)) => return Err(ConfigError::ConflictingPasswordSources),
            (None, None) => return Err(ConfigError::MissingPassword),
        };
        if password.is_empty() {
            return Err(ConfigError::EmptyPassword);
        }
        Ok(Some(Credentials {
            username: username.clone(),
            password,
        }))
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::from_environment()
    }
}

fn resolve_password_command(command: &[String]) -> Result<String, ConfigError> {
    let Some((program, arguments)) = command.split_first() else {
        return Err(ConfigError::PasswordCommandStart);
    };
    if program.is_empty() {
        return Err(ConfigError::PasswordCommandStart);
    }
    let output = Command::new(program)
        .args(arguments)
        .output()
        .map_err(|_| ConfigError::PasswordCommandStart)?;
    if !output.status.success() {
        return Err(ConfigError::PasswordCommandFailed);
    }
    let mut stdout = output.stdout;
    if stdout.ends_with(b"\n") {
        stdout.pop();
        if stdout.ends_with(b"\r") {
            stdout.pop();
        }
    }
    String::from_utf8(stdout).map_err(|_| ConfigError::PasswordNotUtf8)
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

    #[test]
    fn parses_supported_config_values_and_quoted_comments() {
        let mut config = Config::with_paths(PathBuf::from("/tmp/btrn-config"), "vim".into());
        config
            .apply_toml(
                r#"
                server = "news.example.net" # server
                port = 563
                use_tls = true
                username = "reader"
                password_command = ["pass", "show", "news/#account",]
                initial_overview_limit = 5000
                overview_chunk_size = 500
                connect_timeout = 4
                read_timeout = 12
                "#,
            )
            .unwrap();
        assert_eq!(config.server.as_deref(), Some("news.example.net"));
        assert_eq!(config.username.as_deref(), Some("reader"));
        assert_eq!(
            config.password_command.as_deref(),
            Some(
                [
                    "pass".to_owned(),
                    "show".to_owned(),
                    "news/#account".to_owned()
                ]
                .as_slice()
            )
        );
        assert_eq!(config.initial_overview_limit, 5000);
        assert_eq!(config.connect_timeout, Duration::from_secs(4));
        assert_eq!(config.read_timeout, Duration::from_secs(12));
    }

    #[test]
    fn reports_invalid_or_unknown_config_keys_by_line() {
        let mut config = Config::with_paths(PathBuf::from("/tmp/btrn-config"), "vim".into());
        assert!(matches!(
            config.apply_toml("server = 7"),
            Err(ConfigError::InvalidValue {
                line_number: Some(1)
            })
        ));
        assert!(matches!(
            config.apply_toml("mystery = true"),
            Err(ConfigError::UnknownKey {
                line_number: Some(1)
            })
        ));
    }

    fn apply(input: &str) -> Result<Config, ConfigError> {
        let mut config = Config::with_paths(PathBuf::from("/tmp/btrn-config"), "vim".into());
        config.apply_toml(input).map(|()| config)
    }

    #[test]
    fn reports_each_error_category_with_its_line() {
        let prefix = "server = \"news.example.net\"\n# comment\n";
        assert!(matches!(
            apply(&format!("{prefix}server_name = \"x\"\n")),
            Err(ConfigError::UnknownKey {
                line_number: Some(3)
            })
        ));
        assert!(matches!(
            apply(&format!("{prefix}port = \"563\"\n")),
            Err(ConfigError::InvalidValue {
                line_number: Some(3)
            })
        ));
        assert!(matches!(
            apply(&format!("{prefix}username = \n")),
            Err(ConfigError::InvalidLine {
                line_number: Some(3)
            })
        ));
        assert!(matches!(
            apply("[servers]\nhost = \"x\"\n"),
            Err(ConfigError::UnknownKey { .. })
        ));
        assert!(matches!(
            apply("[server]\nhost = \"x\"\n"),
            Err(ConfigError::InvalidValue { .. })
        ));
    }

    #[test]
    fn rejects_out_of_range_numbers_and_duplicate_keys() {
        for input in [
            "port = 0",
            "port = 70000",
            "connect_timeout = -1",
            "use_tls = 1",
        ] {
            assert!(
                matches!(apply(input), Err(ConfigError::InvalidValue { .. })),
                "{input}"
            );
        }
        assert!(apply("server = \"a\"\nserver = \"b\"\n").is_err());
    }

    #[test]
    fn accepts_standard_toml_string_and_array_forms() {
        let config = apply(
            "server = 'news.example.net'\npassword_command = [\n  \"pass\",\n  'show', # entry\n  \"news/\\u00e9\",\n]\n",
        )
        .unwrap();
        assert_eq!(config.server.as_deref(), Some("news.example.net"));
        assert_eq!(
            config.password_command,
            Some(vec!["pass".into(), "show".into(), "news/\u{e9}".into()])
        );
    }

    #[test]
    fn unset_keys_keep_defaults() {
        let config = apply("server = \"news.example.net\"").unwrap();
        assert_eq!(config.port, 563);
        assert!(config.use_tls);
        assert_eq!(config.editor, "vim");
        assert_eq!(config.read_timeout, Duration::from_secs(30));
        assert!(config.password.is_none());
        assert!(apply("").is_ok());
    }

    #[test]
    fn config_errors_never_echo_config_contents() {
        for input in [
            "password = 7\nusername = \"hunter2\"",
            "server = \"hunter2\" = x",
            "port = \"hunter2\"",
            "hunter2 = \"hunter2\"",
        ] {
            let message = apply(input).err().unwrap().to_string();
            assert!(!message.contains("hunter2"), "{message}");
            assert!(!message.contains('7'), "{message}");
        }
    }

    #[test]
    fn resolves_password_command_without_disclosing_failures() {
        let mut config = Config::with_paths(PathBuf::from("/tmp/btrn-config"), "vim".into());
        config.username = Some("reader".into());
        config.password_command = Some(vec!["printf".into(), "secret\\n".into()]);
        let credentials = config.resolve_credentials().unwrap().unwrap();
        assert_eq!(credentials.username, "reader");
        assert_eq!(credentials.password, "secret");
        assert_eq!(
            ConfigError::PasswordCommandFailed.to_string(),
            "password command failed"
        );
    }

    #[test]
    fn loads_config_file_and_reports_missing_or_invalid_password_sources() {
        let directory =
            std::env::temp_dir().join(format!("btrn-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("config.toml"),
            "server = \"news.example.net\"\nusername = \"reader\"\npassword = \"secret\"\n",
        )
        .unwrap();
        let config = Config::load_from_dir(directory.clone(), "vim".into()).unwrap();
        assert_eq!(config.server.as_deref(), Some("news.example.net"));
        assert_eq!(
            config.resolve_credentials().unwrap().unwrap().password,
            "secret"
        );
        std::fs::remove_dir_all(&directory).unwrap();

        let mut config = Config::with_paths(directory, "vim".into());
        config.password = Some("secret".into());
        assert!(matches!(
            config.resolve_credentials(),
            Err(ConfigError::MissingUsername)
        ));
    }
}
