//! Parses and checks the user's edited composition file.

use super::mailbox::{parse_mailbox, Mailbox};
use crate::model::article::is_valid_header_name;

/// Longest allowed line in bytes, excluding the line ending.
pub const MAX_LINE_BYTES: usize = 998;

/// Headers that would change what the article does rather than what it says:
/// cancel or replace other articles, act as a control message, or approve a
/// moderated post. btrn refuses to send them.
const REJECTED_HEADERS: [&str; 5] = [
    "Control",
    "Also-Control",
    "Supersedes",
    "Approved",
    "Cancel-Key",
];

/// The edited composition: non-empty headers in file order and body lines.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Composition {
    pub headers: Vec<(String, String)>,
    pub body: Vec<String>,
}

impl Composition {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The parsed `From` mailbox, if it is present and valid.
    pub fn from(&self) -> Option<Mailbox> {
        self.header("From").and_then(parse_mailbox)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompositionError {
    MissingHeader(&'static str),
    InvalidAddress,
    InvalidNewsgroups(&'static str),
    DuplicateHeader(String),
    RejectedHeader(&'static str),
    InvalidHeaderLine(usize),
    ControlCharacter(usize),
    MissingSeparator,
    LineTooLong(usize),
    /// An ASCII header whose continuation lines join to more than
    /// [`MAX_LINE_BYTES`]; it is posted on one line.
    HeaderTooLong(String),
    NotUtf8,
}

impl std::fmt::Display for CompositionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingHeader(name) => {
                write!(
                    formatter,
                    "ERROR: the \"{name}:\" line is missing from the header."
                )
            }
            Self::InvalidAddress => write!(
                formatter,
                "ERROR: the \"From:\" line does not contain a valid address."
            ),
            Self::InvalidNewsgroups(name) => write!(
                formatter,
                "ERROR: the \"{name}:\" line contains an invalid newsgroup name."
            ),
            Self::DuplicateHeader(name) => {
                write!(
                    formatter,
                    "ERROR: the \"{name}:\" line appears more than once."
                )
            }
            Self::RejectedHeader(name) => {
                write!(
                    formatter,
                    "ERROR: btrn does not send the \"{name}:\" header."
                )
            }
            Self::InvalidHeaderLine(line) => {
                write!(formatter, "ERROR: line {line} is not a valid header line.")
            }
            Self::ControlCharacter(line) => write!(
                formatter,
                "ERROR: line {line} contains a control character."
            ),
            Self::MissingSeparator => write!(
                formatter,
                "ERROR: there is no blank line between the header and the body."
            ),
            Self::LineTooLong(line) => write!(
                formatter,
                "ERROR: line {line} is longer than {MAX_LINE_BYTES} bytes."
            ),
            Self::HeaderTooLong(name) => write!(
                formatter,
                "ERROR: the \"{name}:\" line is longer than {MAX_LINE_BYTES} bytes once its continuation lines are joined."
            ),
            Self::NotUtf8 => write!(formatter, "ERROR: the article body is not valid UTF-8."),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedComposition {
    pub composition: Composition,
    pub errors: Vec<CompositionError>,
}

/// Parses the composition file and checks it for completeness and
/// correctness. A composition with errors must not be sent.
pub fn parse_composition(bytes: &[u8]) -> ParsedComposition {
    let mut errors = Vec::new();
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => {
            errors.push(CompositionError::NotUtf8);
            String::from_utf8_lossy(bytes).into_owned()
        }
    };
    let mut lines = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect::<Vec<_>>();
    if text.ends_with('\n') {
        lines.pop();
    }
    for (index, line) in lines.iter().enumerate() {
        if line.len() > MAX_LINE_BYTES {
            errors.push(CompositionError::LineTooLong(index + 1));
        }
    }

    let separator = lines.iter().position(|line| line.trim().is_empty());
    if separator.is_none() {
        errors.push(CompositionError::MissingSeparator);
    }
    let header_end = separator.unwrap_or(lines.len());

    let mut raw_headers: Vec<(String, String)> = Vec::new();
    for (index, line) in lines[..header_end].iter().enumerate() {
        if line
            .chars()
            .any(|character| character.is_control() && character != '\t')
        {
            errors.push(CompositionError::ControlCharacter(index + 1));
        }
        if line.starts_with([' ', '\t']) {
            match raw_headers.last_mut() {
                Some((_, value)) => {
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(line.trim());
                }
                None => errors.push(CompositionError::InvalidHeaderLine(index + 1)),
            }
            continue;
        }
        match line.split_once(':') {
            Some((name, value)) if is_valid_header_name(name.as_bytes()) => {
                raw_headers.push((name.to_owned(), value.trim().to_owned()));
            }
            _ => errors.push(CompositionError::InvalidHeaderLine(index + 1)),
        }
    }
    // ASCII values are posted unfolded on one line; non-ASCII values are
    // re-folded as encoded words.
    for (name, value) in &raw_headers {
        if value.is_ascii() && name.len() + 2 + value.len() > MAX_LINE_BYTES {
            errors.push(CompositionError::HeaderTooLong(name.clone()));
        }
    }
    let headers = raw_headers
        .into_iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .collect::<Vec<_>>();
    for (index, (name, _)) in headers.iter().enumerate() {
        let first = headers
            .iter()
            .position(|(other, _)| other.eq_ignore_ascii_case(name));
        let repeated = headers[index + 1..]
            .iter()
            .any(|(other, _)| other.eq_ignore_ascii_case(name));
        if first == Some(index) && repeated {
            errors.push(CompositionError::DuplicateHeader(name.clone()));
        }
        if first == Some(index) {
            if let Some(rejected) = REJECTED_HEADERS
                .iter()
                .find(|rejected| rejected.eq_ignore_ascii_case(name))
            {
                errors.push(CompositionError::RejectedHeader(rejected));
            }
        }
    }

    let body = lines
        .get(header_end + 1..)
        .unwrap_or_default()
        .iter()
        .map(|line| (*line).to_owned())
        .collect();
    let composition = Composition { headers, body };

    match composition.header("Newsgroups") {
        None => errors.push(CompositionError::MissingHeader("Newsgroups")),
        Some(groups) if !valid_newsgroups(groups) => {
            errors.push(CompositionError::InvalidNewsgroups("Newsgroups"));
        }
        Some(_) => {}
    }
    if let Some(groups) = composition.header("Followup-To") {
        if !groups.eq_ignore_ascii_case("poster") && !valid_newsgroups(groups) {
            errors.push(CompositionError::InvalidNewsgroups("Followup-To"));
        }
    }
    if composition.header("Subject").is_none() {
        errors.push(CompositionError::MissingHeader("Subject"));
    }
    match composition.header("From") {
        None => errors.push(CompositionError::MissingHeader("From")),
        Some(_) if composition.from().is_none() => errors.push(CompositionError::InvalidAddress),
        Some(_) => {}
    }

    ParsedComposition {
        composition,
        errors,
    }
}

/// The comma-separated group names, with surrounding spaces removed.
pub fn newsgroup_names(value: &str) -> Vec<&str> {
    value.split(',').map(str::trim).collect()
}

fn valid_newsgroups(value: &str) -> bool {
    newsgroup_names(value).iter().all(|name| {
        !name.is_empty()
            && !name.starts_with('.')
            && !name.ends_with('.')
            && !name.contains("..")
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "+-_.".contains(character))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "Newsgroups: comp.test, misc.test\n\
                         Subject: Re: hello\n\
                         Followup-To:\n\
                         From: Example Name <user@example.com>\n\
                         Organization:\n\
                         Keywords: one,\n  two\n\
                         \n\
                         In article <a@b>, P wrote:\n\
                         >quoted\n\
                         \n\
                         reply\n";

    #[test]
    fn parses_headers_folding_and_body_dropping_empty_headers() {
        let parsed = parse_composition(VALID.as_bytes());
        assert_eq!(parsed.errors, []);
        assert_eq!(
            parsed.composition.headers,
            [
                ("Newsgroups".to_owned(), "comp.test, misc.test".to_owned()),
                ("Subject".to_owned(), "Re: hello".to_owned()),
                (
                    "From".to_owned(),
                    "Example Name <user@example.com>".to_owned()
                ),
                ("Keywords".to_owned(), "one, two".to_owned()),
            ]
        );
        assert_eq!(
            parsed.composition.body,
            ["In article <a@b>, P wrote:", ">quoted", "", "reply"]
        );
        assert_eq!(
            newsgroup_names("comp.test, misc.test"),
            ["comp.test", "misc.test"]
        );
    }

    #[test]
    fn accepts_crlf_line_endings() {
        let parsed = parse_composition(VALID.replace('\n', "\r\n").as_bytes());
        assert_eq!(parsed.errors, []);
        assert_eq!(parsed.composition.body.last().unwrap(), "reply");
    }

    fn errors(text: &str) -> Vec<String> {
        parse_composition(text.as_bytes())
            .errors
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn reports_missing_required_headers() {
        assert_eq!(
            errors("Newsgroups:\nSubject:  \nFrom:\n\nbody\n"),
            [
                "ERROR: the \"Newsgroups:\" line is missing from the header.",
                "ERROR: the \"Subject:\" line is missing from the header.",
                "ERROR: the \"From:\" line is missing from the header.",
            ]
        );
    }

    #[test]
    fn reports_invalid_from_newsgroups_and_followup_to() {
        assert_eq!(
            errors("Newsgroups: comp.test,,x\nSubject: s\nFrom: Example Name\nFollowup-To: bad group\n\nb\n"),
            [
                "ERROR: the \"Newsgroups:\" line contains an invalid newsgroup name.",
                "ERROR: the \"Followup-To:\" line contains an invalid newsgroup name.",
                "ERROR: the \"From:\" line does not contain a valid address.",
            ]
        );
        assert_eq!(
            errors("Newsgroups: a.b\nSubject: s\nFrom: u@example.com\nFollowup-To: poster\n\nb\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn reports_duplicates_invalid_lines_and_missing_separator() {
        assert_eq!(
            errors(
                "Newsgroups: a.b\nSubject: s\nsubject: t\nFrom: u@example.com\nnot a header\n\nb\n"
            ),
            [
                "ERROR: line 5 is not a valid header line.",
                "ERROR: the \"Subject:\" line appears more than once.",
            ]
        );
        assert_eq!(
            errors(" orphan\nNewsgroups: a.b\nSubject: s\nFrom: u@example.com\n"),
            [
                "ERROR: there is no blank line between the header and the body.",
                "ERROR: line 1 is not a valid header line.",
            ]
        );
    }

    #[test]
    fn rejects_headers_that_change_what_the_article_does() {
        assert_eq!(
            errors(
                "Newsgroups: a.b\nSubject: s\nFrom: u@example.com\ncontrol: cancel <x@y>\n\
                 Supersedes: <x@y>\nApproved: m@example.com\nAlso-Control: x\nCancel-Key: k\n\nb\n"
            ),
            [
                "ERROR: btrn does not send the \"Control:\" header.",
                "ERROR: btrn does not send the \"Supersedes:\" header.",
                "ERROR: btrn does not send the \"Approved:\" header.",
                "ERROR: btrn does not send the \"Also-Control:\" header.",
                "ERROR: btrn does not send the \"Cancel-Key:\" header.",
            ]
        );
        assert_eq!(
            errors(
                "Newsgroups: a.b\nSubject: s\nFrom: u@example.com\nControl:\nApproved:  \n\nb\n"
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            errors(
                "Newsgroups: a.b\nSubject: s\nFrom: u@example.com\nControl:\n  cancel <x@y>\n\nb\n"
            ),
            ["ERROR: btrn does not send the \"Control:\" header."]
        );
    }

    #[test]
    fn rejects_control_characters_in_header_lines() {
        assert_eq!(
            errors("Newsgroups: a.b\nSubject: s\rX-Bad: y\nFrom: u@example.com\nX-Esc: \u{1b}[2J\n\nb\u{0}\n"),
            [
                "ERROR: line 2 contains a control character.",
                "ERROR: line 4 contains a control character.",
            ]
        );
        assert_eq!(
            errors("Newsgroups: a.b\nSubject:\ts\nFrom: u@example.com\r\n\nb\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn reports_long_lines_and_invalid_utf8() {
        let long = "x".repeat(MAX_LINE_BYTES + 1);
        let text = format!("Newsgroups: a.b\nSubject: s\nFrom: u@example.com\n\nok\n{long}\n");
        assert_eq!(errors(&text), ["ERROR: line 6 is longer than 998 bytes."]);
        let exact = format!(
            "Newsgroups: a.b\nSubject: s\nFrom: u@example.com\n\n{}\n",
            "x".repeat(MAX_LINE_BYTES)
        );
        assert!(errors(&exact).is_empty());

        // Short continuation lines can still join into an over-long line.
        let folded = format!(
            "Newsgroups: a.b\nSubject: s\nFrom: u@example.com\nKeywords: k\n{}\n",
            format!(" {}\n", "w".repeat(90)).repeat(11)
        );
        assert_eq!(
            errors(&folded),
            ["ERROR: the \"Keywords:\" line is longer than 998 bytes once its continuation lines are joined."]
        );
        let fits = format!(
            "Newsgroups: a.b\nSubject: s\nFrom: u@example.com\nKeywords: k\n{}\n",
            format!(" {}\n", "w".repeat(90)).repeat(10)
        );
        assert!(errors(&fits).is_empty());

        let mut bytes = b"Newsgroups: a.b\nSubject: s\nFrom: u@example.com\n\nbad ".to_vec();
        bytes.push(0xff);
        assert_eq!(
            parse_composition(&bytes).errors,
            [CompositionError::NotUtf8]
        );
    }
}
