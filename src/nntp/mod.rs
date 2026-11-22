pub mod active;
mod capabilities;
mod group;
pub mod overview;
pub mod post;

use crate::model::article::MessageId;
use crate::persistence::newsrc::ArticleRange;
pub use capabilities::ServerCapabilities;
pub use group::GroupSelection;
pub use overview::{OverviewError, OverviewFormat, OverviewRows};
pub use post::PostError;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

pub const MAX_LINE_BYTES: usize = 65_536;
pub const MAX_RESPONSE_BYTES: usize = 16_777_216;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NntpResponse {
    pub status: u16,
    pub first_line: Vec<u8>,
    pub data: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidCommand,
    UnexpectedStatus(u16),
    MalformedResponse,
    OversizedLine,
    OversizedResponse,
    MissingTerminator,
    InvalidMultiline,
    UnexpectedGreeting,
    Tls,
    Dns,
    Connection,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NntpError {
    pub protocol: ProtocolError,
}

impl NntpError {
    fn protocol(protocol: ProtocolError) -> Self {
        Self { protocol }
    }
}

impl std::fmt::Display for NntpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}", self.protocol)
    }
}

impl std::error::Error for NntpError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthenticationError {
    Transport(NntpError),
    InvalidArguments,
    AuthenticationFailed(u16),
    ChallengeRequired,
    UnexpectedStatus(u16),
}

impl std::fmt::Display for AuthenticationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "{error}"),
            Self::InvalidArguments => write!(formatter, "invalid authentication arguments"),
            Self::AuthenticationFailed(status) => {
                write!(formatter, "NNTP authentication failed (status {status})")
            }
            Self::ChallengeRequired => write!(formatter, "NNTP authentication challenge required"),
            Self::UnexpectedStatus(status) => {
                write!(formatter, "unexpected authentication status {status}")
            }
        }
    }
}

impl std::error::Error for AuthenticationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleError {
    Transport(NntpError),
    InvalidArticleNumber,
    InvalidMessageId,
    ArticleNotFound { status: u16, response: Vec<u8> },
    UnexpectedStatus { status: u16, response: Vec<u8> },
}

impl std::fmt::Display for ArticleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "{error}"),
            Self::InvalidArticleNumber => write!(formatter, "invalid article number"),
            Self::InvalidMessageId => write!(formatter, "invalid message ID"),
            Self::ArticleNotFound { status: 423, .. } => {
                write!(formatter, "article number not found")
            }
            Self::ArticleNotFound { status: 430, .. } => {
                write!(formatter, "article not found")
            }
            Self::ArticleNotFound { status, .. } => {
                write!(formatter, "article not found (status {status})")
            }
            Self::UnexpectedStatus { status, .. } => {
                write!(formatter, "unexpected article response status {status}")
            }
        }
    }
}

impl std::error::Error for ArticleError {}

enum NetworkStream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Read for NetworkStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for NetworkStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

impl NetworkStream {
    fn shutdown(&self) {
        let socket = match self {
            Self::Plain(stream) => stream,
            Self::Tls(stream) => &stream.sock,
        };
        let _ = socket.shutdown(std::net::Shutdown::Both);
    }
}

pub struct NntpSession {
    stream: BufReader<NetworkStream>,
    capabilities: Option<ServerCapabilities>,
    overview_format: Option<OverviewFormat>,
    selected_group: Option<String>,
    /// Whether the greeting (or a later `MODE READER`) reported `200`
    /// (posting allowed) rather than `201`.
    posting_allowed: bool,
}

impl NntpSession {
    pub fn connect(
        host: &str,
        port: u16,
        use_tls: bool,
        connect_timeout: Duration,
        io_timeout: Duration,
    ) -> Result<Self, NntpError> {
        let mut root_store = RootCertStore::empty();
        let certificates = rustls_native_certs::load_native_certs();
        for certificate in certificates.certs {
            root_store
                .add(certificate)
                .map_err(|_| NntpError::protocol(ProtocolError::Tls))?;
        }
        if root_store.is_empty() {
            return Err(NntpError::protocol(ProtocolError::Tls));
        }

        Self::connect_with_roots(host, port, use_tls, connect_timeout, io_timeout, root_store)
    }

    fn connect_with_roots(
        host: &str,
        port: u16,
        use_tls: bool,
        connect_timeout: Duration,
        io_timeout: Duration,
        root_store: RootCertStore,
    ) -> Result<Self, NntpError> {
        let addresses = (host, port)
            .to_socket_addrs()
            .map_err(|_| NntpError::protocol(ProtocolError::Dns))?;

        let mut last_error = None;
        let mut tcp = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, connect_timeout) {
                Ok(stream) => {
                    tcp = Some(stream);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let tcp = tcp.ok_or_else(|| {
            let _ = last_error;
            NntpError::protocol(ProtocolError::Connection)
        })?;
        tcp.set_read_timeout(Some(io_timeout))
            .map_err(|_| NntpError::protocol(ProtocolError::Io))?;
        tcp.set_write_timeout(Some(io_timeout))
            .map_err(|_| NntpError::protocol(ProtocolError::Io))?;

        let network = if use_tls {
            let config = ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth();
            let server_name = ServerName::try_from(host.to_owned())
                .map_err(|_| NntpError::protocol(ProtocolError::Tls))?;
            let connection = ClientConnection::new(Arc::new(config), server_name)
                .map_err(|_| NntpError::protocol(ProtocolError::Tls))?;
            let mut tls = StreamOwned::new(connection, tcp);
            tls.conn
                .complete_io(&mut tls.sock)
                .map_err(|_| NntpError::protocol(ProtocolError::Tls))?;
            NetworkStream::Tls(Box::new(tls))
        } else {
            NetworkStream::Plain(tcp)
        };

        let mut session = Self {
            stream: BufReader::new(network),
            capabilities: None,
            overview_format: None,
            selected_group: None,
            posting_allowed: false,
        };
        let greeting = session.read_response(false)?;
        let status = greeting.status;
        if status != 200 && status != 201 {
            return Err(NntpError::protocol(ProtocolError::UnexpectedGreeting));
        }
        session.posting_allowed = status == 200;
        Ok(session)
    }

    pub fn execute(&mut self, command: &str, multiline: bool) -> Result<NntpResponse, NntpError> {
        if command.is_empty()
            || command.len() + 2 > MAX_LINE_BYTES
            || command.contains('\r')
            || command.contains('\n')
        {
            return Err(NntpError::protocol(ProtocolError::InvalidCommand));
        }
        self.stream
            .get_mut()
            .write_all(format!("{command}\r\n").as_bytes())
            .map_err(|_| NntpError::protocol(ProtocolError::Io))?;
        self.stream
            .get_mut()
            .flush()
            .map_err(|_| NntpError::protocol(ProtocolError::Io))?;
        self.read_response(multiline)
    }

    pub fn mode_reader(&mut self) -> Result<NntpResponse, NntpError> {
        let response = self.execute("MODE READER", false)?;
        if matches!(response.status, 200 | 201) {
            self.posting_allowed = response.status == 200;
        }
        Ok(response)
    }

    /// Whether the server's greeting allowed posting.
    pub fn posting_allowed(&self) -> bool {
        self.posting_allowed
    }

    fn read_response(&mut self, multiline: bool) -> Result<NntpResponse, NntpError> {
        let mut line = Vec::new();
        self.read_line(&mut line)?;
        if line.last() == Some(&b'\n') {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            } else {
                self.stream.get_ref().shutdown();
                return Err(NntpError::protocol(ProtocolError::MalformedResponse));
            }
        } else {
            self.stream.get_ref().shutdown();
            return Err(NntpError::protocol(ProtocolError::MissingTerminator));
        }

        let Some(status) = parse_status(&line) else {
            self.stream.get_ref().shutdown();
            return Err(NntpError::protocol(ProtocolError::MalformedResponse));
        };
        let first_line = line.clone();
        let mut data = Vec::new();
        let has_multiline_data = multiline && (100..300).contains(&status);
        if has_multiline_data {
            let mut decoded_size = 0usize;
            loop {
                line.clear();
                self.read_line(&mut line).map_err(|error| {
                    if error.protocol == ProtocolError::Connection {
                        NntpError::protocol(ProtocolError::MissingTerminator)
                    } else {
                        error
                    }
                })?;
                if line.as_slice() == b".\r\n" {
                    break;
                }
                if line.last() != Some(&b'\n') {
                    return Err(NntpError::protocol(ProtocolError::MissingTerminator));
                }
                line.pop();
                if line.last() != Some(&b'\r') {
                    self.stream.get_ref().shutdown();
                    return Err(NntpError::protocol(ProtocolError::InvalidMultiline));
                }
                line.pop();

                let mut decoded = std::mem::take(&mut line);
                if decoded.starts_with(b"..") {
                    decoded.remove(0);
                }
                decoded_size = decoded_size
                    .checked_add(decoded.len())
                    .and_then(|size| size.checked_add(2))
                    .ok_or_else(|| {
                        self.stream.get_ref().shutdown();
                        NntpError::protocol(ProtocolError::OversizedResponse)
                    })?;
                if decoded_size > MAX_RESPONSE_BYTES {
                    self.stream.get_ref().shutdown();
                    return Err(NntpError::protocol(ProtocolError::OversizedResponse));
                }
                data.push(decoded);
            }
        }

        Ok(NntpResponse {
            status,
            first_line,
            data,
        })
    }

    fn read_line(&mut self, line: &mut Vec<u8>) -> Result<usize, NntpError> {
        loop {
            let (bytes, complete) = {
                let buffer = self
                    .stream
                    .fill_buf()
                    .map_err(|_| NntpError::protocol(ProtocolError::Io))?;
                if buffer.is_empty() {
                    return Err(NntpError::protocol(ProtocolError::Connection));
                }

                let end = buffer
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(buffer.len(), |index| index + 1);
                let available = MAX_LINE_BYTES + 1 - line.len();
                let copied = end.min(available);
                line.extend_from_slice(&buffer[..copied]);
                (copied, copied == end && buffer[copied - 1] == b'\n')
            };
            self.stream.consume(bytes);
            if line.len() > MAX_LINE_BYTES {
                self.stream.get_ref().shutdown();
                return Err(NntpError::protocol(ProtocolError::OversizedLine));
            }
            if complete {
                return Ok(line.len());
            }
        }
    }

    pub fn capabilities(&mut self) -> Result<NntpResponse, NntpError> {
        self.execute("CAPABILITIES", true)
    }

    pub fn discover_capabilities(&mut self) -> Result<&ServerCapabilities, NntpError> {
        let response = self.capabilities()?;
        if response.status != 101 {
            return Err(NntpError::protocol(ProtocolError::UnexpectedStatus(
                response.status,
            )));
        }
        self.capabilities = Some(ServerCapabilities::parse(&response.data));
        Ok(self
            .capabilities
            .as_ref()
            .expect("capabilities were just set"))
    }

    pub fn server_capabilities(&self) -> Option<&ServerCapabilities> {
        self.capabilities.as_ref()
    }

    pub fn authenticate(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<(), AuthenticationError> {
        if username.is_empty()
            || !username.is_ascii()
            || username
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
            || password.is_empty()
            || password.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(AuthenticationError::InvalidArguments);
        }

        let response = self
            .execute(&format!("AUTHINFO USER {username}"), false)
            .map_err(AuthenticationError::Transport)?;
        match response.status {
            281 => return Ok(()),
            381 => {}
            480 => return Err(AuthenticationError::ChallengeRequired),
            481 | 482 | 502 | 503 => {
                return Err(AuthenticationError::AuthenticationFailed(response.status))
            }
            status => return Err(AuthenticationError::UnexpectedStatus(status)),
        }

        let response = self
            .execute(&format!("AUTHINFO PASS {password}"), false)
            .map_err(AuthenticationError::Transport)?;
        match response.status {
            281 => Ok(()),
            480 => Err(AuthenticationError::ChallengeRequired),
            481 | 482 | 502 | 503 => {
                Err(AuthenticationError::AuthenticationFailed(response.status))
            }
            status => Err(AuthenticationError::UnexpectedStatus(status)),
        }
    }

    pub fn list_active(&mut self) -> Result<NntpResponse, NntpError> {
        self.execute("LIST ACTIVE", true)
    }

    /// The `LIST ACTIVE` posting status of one group, or `None` when the
    /// server does not answer the single-group form or does not list it.
    pub fn group_posting_status(
        &mut self,
        name: &str,
    ) -> Result<Option<active::PostingStatus>, NntpError> {
        if !group::is_valid_group_argument(name) {
            return Err(NntpError::protocol(ProtocolError::InvalidCommand));
        }
        let response = self.execute(&format!("LIST ACTIVE {name}"), true)?;
        if response.status != 215 {
            return Ok(None);
        }
        Ok(active::parse_active_groups(&response.data)
            .groups
            .into_iter()
            .find(|group| group.name == name)
            .map(|group| group.posting_status))
    }

    /// Posts an article given as header lines, a blank line, and body lines
    /// without line endings. Never retried automatically.
    pub fn post(&mut self, article: &[Vec<u8>]) -> Result<(), PostError> {
        let wire = post::encode_article(article)?;
        let response = self.execute("POST", false).map_err(PostError::Transport)?;
        match response.status {
            340 => {}
            440 => {
                return Err(PostError::NotPermitted {
                    response: response.first_line,
                })
            }
            status => {
                return Err(PostError::UnexpectedStatus {
                    status,
                    response: response.first_line,
                })
            }
        }
        let stream = self.stream.get_mut();
        stream
            .write_all(&wire)
            .and_then(|()| stream.flush())
            .map_err(|_| PostError::Transport(NntpError::protocol(ProtocolError::Io)))?;
        let response = self.read_response(false).map_err(PostError::Transport)?;
        match response.status {
            240 => Ok(()),
            441 => Err(PostError::Failed {
                response: response.first_line,
            }),
            status => Err(PostError::UnexpectedStatus {
                status,
                response: response.first_line,
            }),
        }
    }

    pub fn select_group(
        &mut self,
        name: &str,
    ) -> Result<GroupSelection, group::GroupSelectionError> {
        if !group::is_valid_group_argument(name) {
            return Err(group::GroupSelectionError::InvalidName);
        }
        let response = self
            .execute(&format!("GROUP {name}"), false)
            .map_err(group::GroupSelectionError::Transport)?;
        let selection = group::parse_group_response(response, name)?;
        self.selected_group = Some(selection.name.clone());
        Ok(selection)
    }

    pub fn overview_format(&mut self) -> Result<&OverviewFormat, OverviewError> {
        if self.overview_format.is_none() {
            let response = self
                .execute("LIST OVERVIEW.FMT", true)
                .map_err(OverviewError::Transport)?;
            let format = match response.status {
                215 => OverviewFormat::parse(&response.data),
                500..=502 => OverviewFormat::standard(),
                status => return Err(OverviewError::UnexpectedStatus(status)),
            };
            self.overview_format = Some(format);
        }
        Ok(self
            .overview_format
            .as_ref()
            .expect("overview format was just set"))
    }

    pub fn list_overview(&mut self, range: ArticleRange) -> Result<OverviewRows, OverviewError> {
        if range.first == 0 || range.last < range.first {
            return Err(OverviewError::InvalidRange);
        }
        if self.selected_group.is_none() {
            return Err(OverviewError::NoSelectedGroup);
        }
        let capabilities = self
            .capabilities
            .as_ref()
            .ok_or(OverviewError::CapabilitiesNotLoaded)?;
        if !capabilities.supports_overview() {
            return Err(OverviewError::Unsupported);
        }
        let command = capabilities
            .overview_command()
            .ok_or(OverviewError::Unsupported)?;
        let format = self.overview_format()?.clone();
        let response = self
            .execute(&format!("{command} {}-{}", range.first, range.last), true)
            .map_err(OverviewError::Transport)?;
        let rows = overview::ensure_overview_status(response)?;
        Ok(overview::parse_overview_rows(&rows, &format, range))
    }

    pub fn article_headers(&mut self, article_number: u64) -> Result<Vec<Vec<u8>>, ArticleError> {
        self.fetch_article_part("HEAD", article_number, 221)
    }

    pub fn article_body(&mut self, article_number: u64) -> Result<Vec<Vec<u8>>, ArticleError> {
        self.fetch_article_part("BODY", article_number, 222)
    }

    /// The headers of the article with `message_id`, which need not be in
    /// the current group.
    pub fn article_headers_by_id(
        &mut self,
        message_id: &MessageId,
    ) -> Result<Vec<Vec<u8>>, ArticleError> {
        // Message IDs never contain whitespace or controls; requiring ASCII
        // as well keeps the command line printable.
        let id = std::str::from_utf8(message_id.as_bytes())
            .ok()
            .filter(|id| id.is_ascii())
            .ok_or(ArticleError::InvalidMessageId)?;
        self.fetch_article_lines(&format!("HEAD <{id}>"), 221)
    }

    fn fetch_article_part(
        &mut self,
        command: &str,
        article_number: u64,
        success_status: u16,
    ) -> Result<Vec<Vec<u8>>, ArticleError> {
        if article_number == 0 {
            return Err(ArticleError::InvalidArticleNumber);
        }
        self.fetch_article_lines(&format!("{command} {article_number}"), success_status)
    }

    fn fetch_article_lines(
        &mut self,
        command: &str,
        success_status: u16,
    ) -> Result<Vec<Vec<u8>>, ArticleError> {
        let response = self
            .execute(command, true)
            .map_err(ArticleError::Transport)?;
        if matches!(response.status, 423 | 430) {
            return Err(ArticleError::ArticleNotFound {
                status: response.status,
                response: response.first_line,
            });
        }
        if response.status != success_status {
            return Err(ArticleError::UnexpectedStatus {
                status: response.status,
                response: response.first_line,
            });
        }
        Ok(response.data)
    }
}

fn parse_status(line: &[u8]) -> Option<u16> {
    if line.len() < 3
        || !line[..3].iter().all(u8::is_ascii_digit)
        || (line.len() > 3 && line[3] != b' ')
    {
        return None;
    }
    let status = std::str::from_utf8(&line[..3]).ok()?.parse().ok()?;
    Some(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use active::{
        parse_active_groups, ActiveGroup, ActiveGroupList, InvalidActiveLine,
        InvalidActiveLineReason, PostingStatus,
    };
    use group::GroupSelectionError;
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::thread;

    fn fake_server(
        handler: impl FnOnce(TcpStream) + Send + 'static,
    ) -> (SocketAddr, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handler(stream);
        });
        (address, server)
    }

    fn connect(address: SocketAddr) -> Result<NntpSession, NntpError> {
        NntpSession::connect(
            "127.0.0.1",
            address.port(),
            false,
            Duration::from_secs(2),
            Duration::from_secs(5),
        )
    }

    fn read_command(reader: &mut impl BufRead, expected: &str) {
        let mut command = String::new();
        reader.read_line(&mut command).unwrap();
        assert_eq!(command, format!("{expected}\r\n"));
    }

    #[test]
    fn parses_status() {
        assert_eq!(parse_status(b"211 5 1 10 example"), Some(211));
        assert_eq!(parse_status(b"not a response"), None);
    }

    #[test]
    fn accepts_both_reader_greetings() {
        for greeting in [b"200 server ready\r\n".as_slice(), b"201 no posting\r\n"] {
            let (address, server) = fake_server(move |mut stream| {
                stream.write_all(greeting).unwrap();
            });
            let session = connect(address).unwrap();
            drop(session);
            server.join().unwrap();
        }
    }

    #[test]
    fn rejects_unexpected_greeting() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"400 unavailable\r\n").unwrap();
        });
        let error = connect(address).err().unwrap();
        assert_eq!(error.protocol, ProtocolError::UnexpectedGreeting);
        server.join().unwrap();
    }

    #[test]
    fn startup_sends_mode_capabilities_and_list_active() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            read_command(&mut reader, "MODE READER");
            stream
                .write_all(b"500 MODE READER unsupported\r\n")
                .unwrap();

            read_command(&mut reader, "CAPABILITIES");
            stream
                .write_all(b"101 Capability list\r\nMODE-READER\r\nLIST ACTIVE\r\n.\r\n")
                .unwrap();

            read_command(&mut reader, "LIST ACTIVE");
            stream
                .write_all(
                    b"215 Active groups\r\n\
                      misc.test 10 1 y\r\n\
                      comp.test 25 20 m\r\n\
                      legacy.group 40 30 x\r\n\
                      malformed.group bad 1 n\r\n\
                      .\r\n",
                )
                .unwrap();
        });

        let mut session = connect(address).unwrap();
        assert_eq!(session.mode_reader().unwrap().status, 500);

        let capabilities = session.capabilities().unwrap();
        assert_eq!(capabilities.status, 101);
        assert_eq!(
            capabilities.data,
            [b"MODE-READER".to_vec(), b"LIST ACTIVE".to_vec()]
        );

        let active = session.list_active().unwrap();
        assert_eq!(active.status, 215);
        assert_eq!(
            parse_active_groups(&active.data),
            ActiveGroupList {
                groups: [
                    ActiveGroup {
                        name: "misc.test".into(),
                        first_article: 1,
                        last_article: 10,
                        posting_status: PostingStatus::Allowed,
                    },
                    ActiveGroup {
                        name: "comp.test".into(),
                        first_article: 20,
                        last_article: 25,
                        posting_status: PostingStatus::Moderated,
                    },
                    ActiveGroup {
                        name: "legacy.group".into(),
                        first_article: 30,
                        last_article: 40,
                        posting_status: PostingStatus::Unknown(b"x".to_vec()),
                    },
                ]
                .to_vec(),
                invalid_lines: [InvalidActiveLine {
                    line_number: 4,
                    reason: InvalidActiveLineReason::ArticleNumber,
                }]
                .to_vec(),
            }
        );
        server.join().unwrap();
    }

    #[test]
    fn authenticates_after_user_password_challenge() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "AUTHINFO USER reader");
            stream.write_all(b"381 password required\r\n").unwrap();
            read_command(&mut reader, "AUTHINFO PASS secret");
            stream
                .write_all(b"281 authentication accepted\r\n")
                .unwrap();
        });
        let mut session = connect(address).unwrap();
        session.authenticate("reader", "secret").unwrap();
        server.join().unwrap();
    }

    #[test]
    fn accepts_user_only_authentication_and_rejects_bad_credentials() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "AUTHINFO USER reader");
            stream.write_all(b"281 already authenticated\r\n").unwrap();
            read_command(&mut reader, "AUTHINFO USER rejected");
            stream
                .write_all(b"481 authentication rejected\r\n")
                .unwrap();
        });
        let mut session = connect(address).unwrap();
        session.authenticate("reader", "unused").unwrap();
        assert_eq!(
            session.authenticate("rejected", "secret"),
            Err(AuthenticationError::AuthenticationFailed(481))
        );
        assert!(!AuthenticationError::AuthenticationFailed(481)
            .to_string()
            .contains("secret"));
        server.join().unwrap();
    }

    #[test]
    fn rejects_bad_password_and_reports_authentication_challenges() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "AUTHINFO USER reader");
            stream.write_all(b"381 password required\r\n").unwrap();
            read_command(&mut reader, "AUTHINFO PASS bad");
            stream.write_all(b"481 password rejected\r\n").unwrap();
            read_command(&mut reader, "AUTHINFO USER challenged");
            stream
                .write_all(b"480 authentication required\r\n")
                .unwrap();
        });
        let mut session = connect(address).unwrap();
        assert_eq!(
            session.authenticate("reader", "bad"),
            Err(AuthenticationError::AuthenticationFailed(481))
        );
        assert_eq!(
            session.authenticate("challenged", "secret"),
            Err(AuthenticationError::ChallengeRequired)
        );
        server.join().unwrap();
    }

    #[test]
    fn rejects_authentication_arguments_that_could_inject_commands() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
        });
        let mut session = connect(address).unwrap();
        assert_eq!(
            session.authenticate("reader\r\nNOOP", "secret"),
            Err(AuthenticationError::InvalidArguments)
        );
        assert_eq!(
            session.authenticate("reader", "secret\r\nNOOP"),
            Err(AuthenticationError::InvalidArguments)
        );
        assert_eq!(
            session.authenticate("reader", "secret\tvalue"),
            Err(AuthenticationError::InvalidArguments)
        );
        drop(session);
        server.join().unwrap();
    }

    #[test]
    fn selects_group_and_parses_server_article_range() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "GROUP comp.test");
            stream.write_all(b"211 10 101 110 comp.test\r\n").unwrap();
        });

        let mut session = connect(address).unwrap();
        let selection = session.select_group("comp.test").unwrap();
        assert_eq!(selection.name, "comp.test");
        assert_eq!(selection.count, 10);
        assert_eq!(selection.first_article, 101);
        assert_eq!(selection.last_article, 110);
        assert!(selection.count_consistent);
        server.join().unwrap();
    }

    #[test]
    fn fetches_bounded_named_overview_using_advertised_over_command() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            read_command(&mut reader, "CAPABILITIES");
            stream
                .write_all(b"101 capabilities\r\nREADER\r\nOVER\r\n.\r\n")
                .unwrap();

            read_command(&mut reader, "GROUP comp.test");
            stream.write_all(b"211 10 1 10 comp.test\r\n").unwrap();

            read_command(&mut reader, "LIST OVERVIEW.FMT");
            stream
                .write_all(
                    b"215 overview format\r\n\
                      Subject:\r\n\
                      From:\r\n\
                      Date:\r\n\
                      Message-ID:\r\n\
                      References:\r\n\
                      Xref:\r\n\
                      :bytes\r\n\
                      :lines\r\n\
                      .\r\n",
                )
                .unwrap();

            read_command(&mut reader, "OVER 4-6");
            stream
                .write_all(
                    b"224 overview follows\r\n\
                      4\tFirst\tA <a@b>\tdate\t<first@b>\t\tserver comp.test:4\t12\t2\r\n\
                      malformed\trow\r\n\
                      7\tOutside\tA <a@b>\tdate\t<outside@b>\t\tserver comp.test:7\t12\t2\r\n\
                      5\tLast\tA <a@b>\tdate\t<old@b>\t\tserver comp.test:5\t12\t2\r\n\
                      5\tLast duplicate\tA <a@b>\tdate\t<new@b>\t\tserver comp.test:5\t14\t3\r\n\
                      .\r\n",
                )
                .unwrap();
        });

        let mut session = connect(address).unwrap();
        let capabilities = session.discover_capabilities().unwrap();
        assert_eq!(capabilities.overview_command(), Some("OVER"));
        session.select_group("comp.test").unwrap();
        let rows = session
            .list_overview(ArticleRange { first: 4, last: 6 })
            .unwrap();
        assert_eq!(
            rows.articles
                .iter()
                .map(|article| article.article_number)
                .collect::<Vec<_>>(),
            [4, 5]
        );
        assert_eq!(
            rows.articles[1].subject.as_deref(),
            Some(b"Last duplicate".as_slice())
        );
        assert_eq!(
            rows.articles[1].message_id.as_deref(),
            Some(b"new@b".as_slice())
        );
        assert_eq!(rows.articles[0].unknown_fields[0].1, b"server comp.test:4");
        assert_eq!(rows.invalid_rows[0].line_number, 2);
        assert_eq!(rows.out_of_range_rows, [3]);
        server.join().unwrap();
    }

    #[test]
    fn fetches_unnamed_overview_with_xover_and_handles_empty_and_error_responses() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            read_command(&mut reader, "CAPABILITIES");
            stream
                .write_all(b"101 capabilities\r\nXOVER\r\n.\r\n")
                .unwrap();

            read_command(&mut reader, "GROUP comp.test");
            stream.write_all(b"211 10 1 10 comp.test\r\n").unwrap();

            read_command(&mut reader, "LIST OVERVIEW.FMT");
            stream.write_all(b"500 format unavailable\r\n").unwrap();

            read_command(&mut reader, "XOVER 20-25");
            stream.write_all(b"224 overview follows\r\n.\r\n").unwrap();

            read_command(&mut reader, "XOVER 26-30");
            stream.write_all(b"503 temporary failure\r\n").unwrap();
        });

        let mut session = connect(address).unwrap();
        assert_eq!(
            session.discover_capabilities().unwrap().overview_command(),
            Some("XOVER")
        );
        session.select_group("comp.test").unwrap();
        let empty = session
            .list_overview(ArticleRange {
                first: 20,
                last: 25,
            })
            .unwrap();
        assert!(empty.articles.is_empty());
        assert!(empty.invalid_rows.is_empty());
        assert_eq!(
            session.list_overview(ArticleRange {
                first: 26,
                last: 30,
            }),
            Err(OverviewError::UnexpectedStatus(503))
        );
        server.join().unwrap();
    }

    #[test]
    fn fetches_article_headers_and_dot_unstuffs_body_bytes() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            read_command(&mut reader, "HEAD 42");
            stream
                .write_all(
                    b"221 42 article headers follow\r\nSubject: test\r\nX-Raw: \xff\r\n.\r\n",
                )
                .unwrap();

            read_command(&mut reader, "BODY 42");
            stream
                .write_all(b"222 42 article body follows\r\nfirst line\r\n..leading dot\r\n\xffraw\r\n.\r\n")
                .unwrap();
        });

        let mut session = connect(address).unwrap();
        assert_eq!(
            session.article_headers(42).unwrap(),
            [b"Subject: test".to_vec(), b"X-Raw: \xff".to_vec()]
        );
        assert_eq!(
            session.article_body(42).unwrap(),
            [
                b"first line".to_vec(),
                b".leading dot".to_vec(),
                b"\xffraw".to_vec(),
            ]
        );
        server.join().unwrap();
    }

    #[test]
    fn distinguishes_missing_articles_and_rejects_article_zero() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            read_command(&mut reader, "HEAD 10");
            stream.write_all(b"423 no such article number\r\n").unwrap();

            read_command(&mut reader, "BODY 11");
            stream
                .write_all(b"430 no article with that message-id\r\n")
                .unwrap();
        });

        let mut session = connect(address).unwrap();
        assert_eq!(
            session.article_headers(0),
            Err(ArticleError::InvalidArticleNumber)
        );
        assert_eq!(
            session.article_headers(10),
            Err(ArticleError::ArticleNotFound {
                status: 423,
                response: b"423 no such article number".to_vec(),
            })
        );
        assert_eq!(
            session.article_body(11),
            Err(ArticleError::ArticleNotFound {
                status: 430,
                response: b"430 no article with that message-id".to_vec(),
            })
        );
        server.join().unwrap();
    }

    #[test]
    fn fetches_headers_by_message_id_and_reports_expired_articles() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "HEAD <parent@example>");
            stream
                .write_all(
                    b"221 0 <parent@example>\r\n\
                      Message-ID: <parent@example>\r\n\
                      Xref: news.example comp.test:7\r\n\
                      .\r\n",
                )
                .unwrap();
            read_command(&mut reader, "HEAD <gone@example>");
            stream
                .write_all(b"430 no article with that message-id\r\n")
                .unwrap();
        });

        let mut session = connect(address).unwrap();
        let id = |value: &[u8]| crate::model::article::MessageId::parse(value).unwrap();
        assert_eq!(
            session
                .article_headers_by_id(&id(b"<parent@example>"))
                .unwrap(),
            [
                b"Message-ID: <parent@example>".to_vec(),
                b"Xref: news.example comp.test:7".to_vec()
            ]
        );
        assert_eq!(
            session.article_headers_by_id(&id(b"<gone@example>")),
            Err(ArticleError::ArticleNotFound {
                status: 430,
                response: b"430 no article with that message-id".to_vec(),
            })
        );
        assert_eq!(
            session.article_headers_by_id(&id("<caf\u{e9}@example>".as_bytes())),
            Err(ArticleError::InvalidMessageId)
        );
        server.join().unwrap();
    }

    #[test]
    fn plans_newest_unread_group_ranges_and_requests_bounded_chunks() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());

            read_command(&mut reader, "CAPABILITIES");
            stream
                .write_all(b"101 capabilities\r\nOVER\r\n.\r\n")
                .unwrap();

            read_command(&mut reader, "GROUP comp.test");
            stream.write_all(b"211 10 1 10 comp.test\r\n").unwrap();

            read_command(&mut reader, "LIST OVERVIEW.FMT");
            stream.write_all(b"500 no format\r\n").unwrap();

            read_command(&mut reader, "OVER 5-6");
            stream
                .write_all(b"224 overview\r\n5\tFive\r\n6\tSix\r\n.\r\n")
                .unwrap();

            read_command(&mut reader, "OVER 9-10");
            stream
                .write_all(b"224 overview\r\n9\tNine\r\n10\tTen\r\n.\r\n")
                .unwrap();
        });

        let mut session = connect(address).unwrap();
        session.discover_capabilities().unwrap();
        let group = session.select_group("comp.test").unwrap();
        let newsrc = crate::persistence::newsrc::NewsrcState::parse("comp.test: 1-4,7-8");
        let ranges = crate::persistence::newsrc::plan_unread_chunks(
            group.first_article,
            group.last_article,
            &newsrc.state.entry("comp.test").unwrap().read_ranges,
            4,
            2,
        );
        assert_eq!(
            ranges,
            [
                ArticleRange { first: 5, last: 6 },
                ArticleRange { first: 9, last: 10 },
            ]
        );

        let mut articles = Vec::new();
        for range in ranges {
            articles.extend(session.list_overview(range).unwrap().articles);
        }
        assert_eq!(
            articles
                .iter()
                .map(|article| article.article_number)
                .collect::<Vec<_>>(),
            [5, 6, 9, 10]
        );
        server.join().unwrap();
    }

    #[test]
    fn rejects_overview_requests_without_capabilities_or_valid_range() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "GROUP comp.test");
            stream.write_all(b"211 1 1 1 comp.test\r\n").unwrap();
        });
        let mut session = connect(address).unwrap();
        session.select_group("comp.test").unwrap();
        assert_eq!(
            session.list_overview(ArticleRange { first: 1, last: 1 }),
            Err(OverviewError::CapabilitiesNotLoaded)
        );
        assert_eq!(
            session.list_overview(ArticleRange { first: 0, last: 1 }),
            Err(OverviewError::InvalidRange)
        );
        server.join().unwrap();
    }

    #[test]
    fn reports_unknown_group_without_constructing_a_selection() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "GROUP missing.group");
            stream
                .write_all(b"411 no such group available\r\n")
                .unwrap();
        });

        let mut session = connect(address).unwrap();
        assert_eq!(
            session.select_group("missing.group"),
            Err(GroupSelectionError::UnknownGroup {
                response: b"411 no such group available".to_vec(),
            })
        );
        server.join().unwrap();
    }

    #[test]
    fn rejects_group_names_that_could_inject_commands() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
        });
        let mut session = connect(address).unwrap();
        assert_eq!(
            session.select_group("comp.test\r\nLIST ACTIVE"),
            Err(GroupSelectionError::InvalidName)
        );
        drop(session);
        server.join().unwrap();
    }

    #[test]
    fn preserves_bytes_and_unstuffs_multiline_data() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "ARTICLE");
            stream
                .write_all(b"220 article follows\r\n..leading dot\r\n\xffraw\r\n.\r\n")
                .unwrap();
        });

        let mut session = connect(address).unwrap();
        let response = session.execute("ARTICLE", true).unwrap();
        assert_eq!(response.status, 220);
        assert_eq!(response.first_line, b"220 article follows");
        assert_eq!(
            response.data,
            [b".leading dot".to_vec(), b"\xffraw".to_vec()]
        );
        server.join().unwrap();
    }

    #[test]
    fn returns_server_error_status_without_reclassifying_it() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "LIST ACTIVE");
            stream.write_all(b"411 no such group\r\n").unwrap();
        });

        let mut session = connect(address).unwrap();
        let response = session.execute("LIST ACTIVE", false).unwrap();
        assert_eq!(response.status, 411);
        assert_eq!(response.first_line, b"411 no such group");
        server.join().unwrap();
    }

    #[test]
    fn rejects_malformed_line_endings() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\n").unwrap();
        });
        let error = connect(address).err().unwrap();
        assert_eq!(error.protocol, ProtocolError::MalformedResponse);
        server.join().unwrap();
    }

    #[test]
    fn rejects_non_numeric_status_lines() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"abc not an NNTP response\r\n").unwrap();
        });
        let error = connect(address).err().unwrap();
        assert_eq!(error.protocol, ProtocolError::MalformedResponse);
        server.join().unwrap();
    }

    #[test]
    fn reports_disconnect_before_greeting() {
        let (address, server) = fake_server(|_stream| {});
        let error = connect(address).err().unwrap();
        assert_eq!(error.protocol, ProtocolError::Connection);
        server.join().unwrap();
    }

    #[test]
    fn rejects_commands_that_could_inject_a_second_command() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
        });
        let mut session = connect(address).unwrap();
        let error = session.execute("QUIT\r\nLIST ACTIVE", false).unwrap_err();
        assert_eq!(error.protocol, ProtocolError::InvalidCommand);
        drop(session);
        server.join().unwrap();
    }

    #[test]
    fn rejects_oversized_lines_and_closes_the_connection() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "LIST ACTIVE");
            let mut response = b"215 groups\r\n".to_vec();
            response.extend(std::iter::repeat_n(b'x', MAX_LINE_BYTES));
            response.extend_from_slice(b"\r\n");
            let _ = stream.write_all(&response);
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
        });

        let mut session = connect(address).unwrap();
        let error = session.list_active().unwrap_err();
        assert_eq!(error.protocol, ProtocolError::OversizedLine);
        assert!(session.execute("NOOP", false).is_err());
        server.join().unwrap();
    }

    #[test]
    fn rejects_oversized_multiline_responses_and_closes_the_connection() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "LIST ACTIVE");

            let mut response = b"215 groups\r\n".to_vec();
            let line = vec![b'x'; 60_000];
            for _ in 0..=MAX_RESPONSE_BYTES / (line.len() + 2) {
                response.extend_from_slice(&line);
                response.extend_from_slice(b"\r\n");
            }
            let _ = stream.write_all(&response);
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
        });

        let mut session = connect(address).unwrap();
        let error = session.list_active().unwrap_err();
        assert_eq!(error.protocol, ProtocolError::OversizedResponse);
        assert!(session.execute("NOOP", false).is_err());
        server.join().unwrap();
    }

    #[test]
    fn reports_missing_multiline_terminator() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "LIST ACTIVE");
            stream
                .write_all(b"215 groups\r\nmisc.test 2 1 y\r\n")
                .unwrap();
            stream.shutdown(std::net::Shutdown::Both).unwrap();
        });

        let mut session = connect(address).unwrap();
        let error = session.list_active().unwrap_err();
        assert_eq!(error.protocol, ProtocolError::MissingTerminator);
        server.join().unwrap();
    }

    #[test]
    fn rejects_multiline_data_without_crlf() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "LIST ACTIVE");
            stream
                .write_all(b"215 groups\r\nmisc.test 2 1 y\n")
                .unwrap();
        });

        let mut session = connect(address).unwrap();
        let error = session.list_active().unwrap_err();
        assert_eq!(error.protocol, ProtocolError::InvalidMultiline);
        server.join().unwrap();
    }

    #[test]
    fn enforces_read_timeout_for_each_response() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 server ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "NOOP");
            thread::sleep(Duration::from_millis(150));
        });
        let mut session = NntpSession::connect(
            "127.0.0.1",
            address.port(),
            false,
            Duration::from_secs(2),
            Duration::from_millis(30),
        )
        .unwrap();
        let error = session.execute("NOOP", false).unwrap_err();
        assert_eq!(error.protocol, ProtocolError::Io);
        server.join().unwrap();
    }

    #[test]
    fn does_not_downgrade_tls_after_a_plaintext_greeting() {
        let (address, server) = fake_server(|mut stream| {
            let _ = stream.write_all(b"200 plaintext server\r\n");
        });
        let error = NntpSession::connect(
            "127.0.0.1",
            address.port(),
            true,
            Duration::from_secs(2),
            Duration::from_secs(2),
        )
        .err()
        .unwrap();
        assert_eq!(error.protocol, ProtocolError::Tls);
        server.join().unwrap();
    }

    #[test]
    fn connects_over_tls_and_exchanges_a_command() {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
        use rustls::{ServerConfig, ServerConnection};

        let certificate = include_bytes!("../../tests/fixtures/nntp/tls-test-cert.der").to_vec();
        let private_key = include_bytes!("../../tests/fixtures/nntp/tls-test-key.der").to_vec();
        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificate.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(private_key)),
            )
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let connection = ServerConnection::new(Arc::new(server_config)).unwrap();
            let mut reader = BufReader::new(StreamOwned::new(connection, stream));
            reader
                .get_mut()
                .write_all(b"200 secure server ready\r\n")
                .unwrap();
            read_command(&mut reader, "NOOP");
            reader
                .get_mut()
                .write_all(b"250 command completed\r\n")
                .unwrap();
        });

        let mut roots = RootCertStore::empty();
        roots.add(CertificateDer::from(certificate)).unwrap();
        let mut session = NntpSession::connect_with_roots(
            "127.0.0.1",
            address.port(),
            true,
            Duration::from_secs(2),
            Duration::from_secs(2),
            roots,
        )
        .unwrap();
        let response = session.execute("NOOP", false).unwrap();
        assert_eq!(response.status, 250);
        server.join().unwrap();
    }

    #[test]
    fn records_whether_the_greeting_allows_posting() {
        for (greeting, allowed) in [
            (b"200 server ready\r\n".as_slice(), true),
            (b"201 no posting\r\n", false),
        ] {
            let (address, server) = fake_server(move |mut stream| {
                stream.write_all(greeting).unwrap();
            });
            let session = connect(address).unwrap();
            assert_eq!(session.posting_allowed(), allowed);
            drop(session);
            server.join().unwrap();
        }
    }

    #[test]
    fn mode_reader_updates_posting_permission() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 transit ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "MODE READER");
            stream.write_all(b"201 reader, no posting\r\n").unwrap();
        });
        let mut session = connect(address).unwrap();
        assert!(session.posting_allowed());
        session.mode_reader().unwrap();
        assert!(!session.posting_allowed());
        server.join().unwrap();
    }

    #[test]
    fn reads_one_groups_posting_status() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "LIST ACTIVE comp.test");
            stream
                .write_all(b"215 list follows\r\ncomp.test 25 20 n\r\n.\r\n")
                .unwrap();
            read_command(&mut reader, "LIST ACTIVE misc.test");
            stream.write_all(b"501 syntax error\r\n").unwrap();
        });
        let mut session = connect(address).unwrap();
        assert_eq!(
            session.group_posting_status("comp.test").unwrap(),
            Some(PostingStatus::Prohibited)
        );
        assert_eq!(session.group_posting_status("misc.test").unwrap(), None);
        assert!(session.group_posting_status("bad group").is_err());
        server.join().unwrap();
    }

    fn sample_post() -> Vec<Vec<u8>> {
        vec![
            b"Newsgroups: comp.test".to_vec(),
            b"Subject: hello".to_vec(),
            Vec::new(),
            b".dotted".to_vec(),
            b"plain".to_vec(),
        ]
    }

    #[test]
    fn posts_a_dot_stuffed_article() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "POST");
            stream.write_all(b"340 send article\r\n").unwrap();
            let mut received = Vec::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == ".\r\n" {
                    break;
                }
                received.push(line);
            }
            assert_eq!(
                received,
                [
                    "Newsgroups: comp.test\r\n",
                    "Subject: hello\r\n",
                    "\r\n",
                    "..dotted\r\n",
                    "plain\r\n"
                ]
            );
            stream.write_all(b"240 article received\r\n").unwrap();
        });
        let mut session = connect(address).unwrap();
        session.post(&sample_post()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn reports_post_not_permitted_and_rejected() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "POST");
            stream.write_all(b"440 posting not permitted\r\n").unwrap();
            read_command(&mut reader, "POST");
            stream.write_all(b"340 send article\r\n").unwrap();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == ".\r\n" {
                    break;
                }
            }
            stream.write_all(b"441 duplicate article\r\n").unwrap();
        });
        let mut session = connect(address).unwrap();
        let error = session.post(&sample_post()).unwrap_err();
        assert_eq!(
            error,
            PostError::NotPermitted {
                response: b"440 posting not permitted".to_vec()
            }
        );
        assert_eq!(
            error.to_string(),
            "posting not permitted: 440 posting not permitted"
        );
        assert_eq!(
            session.post(&sample_post()).unwrap_err(),
            PostError::Failed {
                response: b"441 duplicate article".to_vec()
            }
        );
        server.join().unwrap();
    }

    #[test]
    fn reports_disconnect_during_article_transfer() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 ready\r\n").unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            read_command(&mut reader, "POST");
            stream.write_all(b"340 send article\r\n").unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            drop(reader);
            drop(stream);
        });
        let mut session = connect(address).unwrap();
        assert!(matches!(
            session.post(&sample_post()),
            Err(PostError::Transport(_))
        ));
        server.join().unwrap();
    }

    #[test]
    fn rejects_invalid_article_lines_before_sending_post() {
        let (address, server) = fake_server(|mut stream| {
            stream.write_all(b"200 ready\r\n").unwrap();
        });
        let mut session = connect(address).unwrap();
        assert_eq!(
            session.post(&[b"Subject: a\r\nInjected: b".to_vec()]),
            Err(PostError::InvalidArticle)
        );
        server.join().unwrap();
    }
}
