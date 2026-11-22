//! `POST` support: typed failures and the dot-stuffed wire form of an
//! article.

use super::NntpError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostError {
    Transport(NntpError),
    /// An article line contained CR or LF, so it cannot be framed.
    InvalidArticle,
    /// `440`: posting is not permitted.
    NotPermitted {
        response: Vec<u8>,
    },
    /// `441`: the server rejected the article.
    Failed {
        response: Vec<u8>,
    },
    UnexpectedStatus {
        status: u16,
        response: Vec<u8>,
    },
}

impl std::fmt::Display for PostError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => {
                write!(formatter, "posting failed: connection error ({error})")
            }
            Self::InvalidArticle => {
                write!(formatter, "posting failed: the article has an invalid line")
            }
            Self::NotPermitted { response } => {
                write!(
                    formatter,
                    "posting not permitted: {}",
                    display_response(response)
                )
            }
            Self::Failed { response } => {
                write!(formatter, "posting failed: {}", display_response(response))
            }
            Self::UnexpectedStatus { status, response } => write!(
                formatter,
                "posting failed (status {status}): {}",
                display_response(response)
            ),
        }
    }
}

impl std::error::Error for PostError {}

/// The server's response line made safe to show on one terminal line. This
/// may be shown to the user but must not be logged.
pub fn display_response(response: &[u8]) -> String {
    const MAX_CHARS: usize = 200;
    String::from_utf8_lossy(response)
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'
            } else {
                character
            }
        })
        .take(MAX_CHARS)
        .collect()
}

/// Frames article lines for the wire: CRLF endings, an extra `.` on lines
/// beginning with `.`, and the terminating `.` line.
pub fn encode_article(lines: &[Vec<u8>]) -> Result<Vec<u8>, PostError> {
    let mut wire = Vec::new();
    for line in lines {
        if line.iter().any(|byte| matches!(byte, b'\r' | b'\n')) {
            return Err(PostError::InvalidArticle);
        }
        if line.starts_with(b".") {
            wire.push(b'.');
        }
        wire.extend_from_slice(line);
        wire.extend_from_slice(b"\r\n");
    }
    wire.extend_from_slice(b".\r\n");
    Ok(wire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_stuffs_and_terminates_article() {
        let wire = encode_article(&[
            b"Subject: s".to_vec(),
            Vec::new(),
            b".leading dot".to_vec(),
            b".".to_vec(),
            b"plain".to_vec(),
        ])
        .unwrap();
        assert_eq!(
            wire,
            b"Subject: s\r\n\r\n..leading dot\r\n..\r\nplain\r\n.\r\n"
        );
    }

    #[test]
    fn rejects_lines_with_embedded_line_breaks() {
        assert_eq!(
            encode_article(&[b"a\rb".to_vec()]),
            Err(PostError::InvalidArticle)
        );
        assert_eq!(
            encode_article(&[b"a\nb".to_vec()]),
            Err(PostError::InvalidArticle)
        );
    }

    #[test]
    fn response_text_is_single_line_and_sanitized() {
        assert_eq!(
            display_response(b"441 bad\x1b[31m header"),
            "441 bad\u{fffd}[31m header"
        );
        assert_eq!(display_response(&[b'x'; 500]).len(), 200);
    }
}
