pub mod active;
mod group;

use group::GroupSelection;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

pub const MAX_LINE_BYTES: usize = 65_536;
pub const MAX_RESPONSE_BYTES: usize = 16_777_216;
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NntpResponse {
    pub status: u16,
    pub first_line: Vec<u8>,
    pub data: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    InvalidCommand,
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

enum NetworkStream {
    Plain(TcpStream),
    Tls(StreamOwned<ClientConnection, TcpStream>),
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
            NetworkStream::Tls(tls)
        } else {
            NetworkStream::Plain(tcp)
        };

        let mut session = Self {
            stream: BufReader::new(network),
        };
        let greeting = session.read_response(false)?;
        let status = greeting.status;
        if status != 200 && status != 201 {
            return Err(NntpError::protocol(ProtocolError::UnexpectedGreeting));
        }
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
        self.execute("MODE READER", false)
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
        if multiline {
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

    pub fn list_active(&mut self) -> Result<NntpResponse, NntpError> {
        self.execute("LIST ACTIVE", true)
    }

    pub fn select_group(
        &mut self,
        name: &str,
    ) -> Result<GroupSelection, group::GroupSelectionError> {
        if name.is_empty()
            || !name.is_ascii()
            || name
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            return Err(group::GroupSelectionError::InvalidName);
        }
        let response = self
            .execute(&format!("GROUP {name}"), false)
            .map_err(group::GroupSelectionError::Transport)?;
        group::parse_group_response(response, name)
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
}
