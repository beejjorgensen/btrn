//! Builds the article lines sent with `POST` from a checked composition.

use super::composition::{newsgroup_names, Composition, MAX_LINE_BYTES};
use super::mailbox::Mailbox;
use crate::model::article::MessageId;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::time::{SystemTime, UNIX_EPOCH};

pub const USER_AGENT: &str = "btrn";

/// Headers btrn sets itself, or that only the server may add; copies in the
/// composition are ignored. Every `Content-` header is ignored as well, since
/// btrn fixes the article's MIME type.
const PROTECTED_HEADERS: [&str; 16] = [
    "Message-ID",
    "Date",
    "Path",
    "User-Agent",
    "References",
    "In-Reply-To",
    "MIME-Version",
    "Xref",
    "Lines",
    "Bytes",
    "Injection-Info",
    "Injection-Date",
    "NNTP-Posting-Host",
    "NNTP-Posting-Date",
    "X-Trace",
    "X-Complaints-To",
];

/// Whether btrn drops a composition header named `name`.
fn is_protected(name: &str) -> bool {
    PROTECTED_HEADERS
        .iter()
        .chain(&ORDERED_HEADERS)
        .any(|known| known.eq_ignore_ascii_case(name))
        || name
            .get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("Content-"))
}

/// Headers that are emitted in a fixed position rather than passed through.
const ORDERED_HEADERS: [&str; 4] = ["From", "Newsgroups", "Subject", "Followup-To"];

/// UTF-8 bytes per encoded word: 45 bytes become 60 base64 characters, and
/// with the `=?UTF-8?B?` and `?=` delimiters a word stays within RFC 2047's
/// 75-character limit.
const ENCODED_WORD_BYTES: usize = 45;

/// Values generated once per composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingContext<'a> {
    pub references: &'a [MessageId],
    /// The complete message ID, including angle brackets.
    pub message_id: &'a str,
    pub date: &'a str,
}

/// The article as header lines, a blank line, and body lines, without line
/// endings. The composition must have passed its check.
pub fn build_article(composition: &Composition, context: &OutgoingContext<'_>) -> Vec<Vec<u8>> {
    let mut lines: Vec<String> = Vec::new();
    let from = composition
        .from()
        .expect("checked composition has a valid From");
    lines.push(format!("From: {}", from.header_value()));
    if let Some(groups) = composition.header("Newsgroups") {
        lines.push(format!("Newsgroups: {}", newsgroup_names(groups).join(",")));
    }
    if let Some(subject) = composition.header("Subject") {
        lines.extend(unstructured_header("Subject", subject));
    }
    if let Some(groups) = composition.header("Followup-To") {
        lines.push(format!(
            "Followup-To: {}",
            newsgroup_names(groups).join(",")
        ));
    }
    if let Some(references) = references_header(context.references) {
        lines.push(references);
    }
    lines.push(format!("Message-ID: {}", context.message_id));
    lines.push(format!("Date: {}", context.date));
    for (name, value) in &composition.headers {
        if !is_protected(name) {
            lines.extend(unstructured_header(name, value));
        }
    }
    lines.push(format!("User-Agent: {USER_AGENT}"));
    lines.push("MIME-Version: 1.0".to_owned());
    lines.push("Content-Type: text/plain; charset=utf-8".to_owned());
    lines.push("Content-Transfer-Encoding: 8bit".to_owned());
    lines.push(String::new());
    lines.extend(composition.body.iter().cloned());
    lines.into_iter().map(String::into_bytes).collect()
}

/// A header line, with non-ASCII values as RFC 2047 encoded-words folded one
/// word per line.
fn unstructured_header(name: &str, value: &str) -> Vec<String> {
    if value.is_ascii() {
        return vec![format!("{name}: {value}")];
    }
    encoded_words(value)
        .into_iter()
        .enumerate()
        .map(|(index, word)| {
            if index == 0 {
                format!("{name}: {word}")
            } else {
                format!(" {word}")
            }
        })
        .collect()
}

/// Encodes text as UTF-8 base64 RFC 2047 encoded-words, splitting only on
/// character boundaries.
pub fn encoded_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut chunk = String::new();
    for character in text.chars() {
        if chunk.len() + character.len_utf8() > ENCODED_WORD_BYTES {
            words.push(encode_word(&chunk));
            chunk.clear();
        }
        chunk.push(character);
    }
    if !chunk.is_empty() || words.is_empty() {
        words.push(encode_word(&chunk));
    }
    words
}

fn encode_word(text: &str) -> String {
    format!("=?UTF-8?B?{}?=", STANDARD.encode(text))
}

/// `References` with the original's references and message ID. When too
/// long it keeps the first ID and as many of the most recent as fit, as in
/// RFC 5537 section 3.4.4.
fn references_header(references: &[MessageId]) -> Option<String> {
    let ids = references
        .iter()
        .map(|id| format!("<{}>", String::from_utf8_lossy(id.as_bytes())))
        .collect::<Vec<_>>();
    let (first, rest) = ids.split_first()?;
    let mut kept = rest.to_vec();
    loop {
        let mut line = format!("References: {first}");
        for id in &kept {
            line.push(' ');
            line.push_str(id);
        }
        if line.len() <= MAX_LINE_BYTES || kept.is_empty() {
            return Some(line);
        }
        kept.remove(0);
    }
}

/// A new message ID whose domain is the From address's domain, falling back
/// to the host name.
pub fn generate_message_id(from: &Mailbox) -> String {
    let domain = Some(from.domain())
        .filter(|domain| !domain.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .filter(|name| valid_domain(name))
        })
        .unwrap_or_else(|| "localhost".to_owned());
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    format!("<{seconds}.{}@{domain}>", super::random_token())
}

fn valid_domain(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-.".contains(character))
}

/// The `Date` header value for `time`, in RFC 5322 format.
pub fn format_date(time: SystemTime) -> String {
    let seconds = time.duration_since(UNIX_EPOCH).map_or(0, |elapsed| {
        i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
    });
    chrono::DateTime::from_timestamp(seconds, 0)
        .unwrap_or_default()
        .to_rfc2822()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::decode_header_value;
    use crate::posting::composition::parse_composition;
    use std::time::Duration;

    fn composition(text: &str) -> Composition {
        let parsed = parse_composition(text.as_bytes());
        assert_eq!(parsed.errors, []);
        parsed.composition
    }

    fn ids(values: &[&str]) -> Vec<MessageId> {
        values
            .iter()
            .map(|value| MessageId::parse(value.as_bytes()).unwrap())
            .collect()
    }

    fn text(lines: &[Vec<u8>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| String::from_utf8(line.clone()).unwrap())
            .collect()
    }

    #[test]
    fn builds_the_specified_headers_and_body() {
        let composition = composition(
            "Newsgroups: comp.test , misc.test\n\
             Subject: Re: Previous Subject\n\
             Followup-To:\n\
             From: Example Name <user@example.com>\n\
             Organization: Yoyodyne Industries\n\
             Keywords:\n\
             Message-ID: <forged@example.com>\n\
             X-Extra: kept\n\
             xref: news.example comp.test:1\n\
             Lines: 3\n\
             In-Reply-To: <forged@example.com>\n\
             Injection-Info: forged\n\
             content-disposition: attachment\n\
             Content-Language: en\n\
             X-Content-Note: kept\n\
             \n\
             In article <a@b>, P wrote:\n\
             >quoted\n\
             .reply\n",
        );
        let references = ids(&["<root@x>", "<a@b>"]);
        let lines = build_article(
            &composition,
            &OutgoingContext {
                references: &references,
                message_id: "<new@example.com>",
                date: "Sun, 22 Nov 2026 22:11:11 +0000",
            },
        );
        assert_eq!(
            text(&lines),
            [
                "From: \"Example Name\" <user@example.com>",
                "Newsgroups: comp.test,misc.test",
                "Subject: Re: Previous Subject",
                "References: <root@x> <a@b>",
                "Message-ID: <new@example.com>",
                "Date: Sun, 22 Nov 2026 22:11:11 +0000",
                "Organization: Yoyodyne Industries",
                "X-Extra: kept",
                "X-Content-Note: kept",
                "User-Agent: btrn",
                "MIME-Version: 1.0",
                "Content-Type: text/plain; charset=utf-8",
                "Content-Transfer-Encoding: 8bit",
                "",
                "In article <a@b>, P wrote:",
                ">quoted",
                ".reply",
            ]
        );
    }

    #[test]
    fn includes_followup_to_and_omits_empty_references() {
        let composition = composition(
            "Newsgroups: a.b\nSubject: s\nFrom: u@example.com\nFollowup-To: poster\n\nbody\n",
        );
        let lines = text(&build_article(
            &composition,
            &OutgoingContext {
                references: &[],
                message_id: "<m@example.com>",
                date: "d",
            },
        ));
        assert!(lines.contains(&"Followup-To: poster".to_owned()));
        assert!(!lines.iter().any(|line| line.starts_with("References:")));
        assert!(!lines.iter().any(|line| line.starts_with("Organization:")));
    }

    #[test]
    fn encodes_non_ascii_headers_as_folded_encoded_words() {
        let subject = "Ünïcödé ".repeat(10);
        let lines = unstructured_header("Subject", &subject);
        assert!(lines.len() > 1);
        assert!(lines[0].starts_with("Subject: =?UTF-8?B?"));
        assert!(lines[1..]
            .iter()
            .all(|line| line.starts_with(" =?UTF-8?B?")));
        let words = lines
            .iter()
            .map(|line| line.trim_start_matches("Subject: ").trim())
            .collect::<Vec<_>>();
        assert!(words.iter().all(|word| word.len() <= 75));
        let unfolded = words.join(" ");
        assert_eq!(
            decode_header_value(b"Subject", unfolded.as_bytes()).0,
            subject
        );
        assert_eq!(unstructured_header("Subject", "plain"), ["Subject: plain"]);
    }

    #[test]
    fn trims_long_references_keeping_the_first_and_most_recent() {
        let values = (0..40)
            .map(|index| format!("<reference-number-{index:03}@news.example.org>"))
            .collect::<Vec<_>>();
        let references = ids(&values.iter().map(String::as_str).collect::<Vec<_>>());
        let line = references_header(&references).unwrap();
        assert!(line.len() <= MAX_LINE_BYTES);
        assert!(line.starts_with("References: <reference-number-000@"));
        assert!(line.ends_with("<reference-number-039@news.example.org>"));
        assert!(!line.contains("<reference-number-001@"));
        assert_eq!(references_header(&[]), None);
    }

    #[test]
    fn message_ids_use_the_from_domain_and_are_unique() {
        let from = Mailbox {
            name: None,
            address: "user@example.com".to_owned(),
        };
        let first = generate_message_id(&from);
        let second = generate_message_id(&from);
        assert!(first.starts_with('<') && first.ends_with("@example.com>"));
        assert!(MessageId::parse(first.as_bytes()).is_some());
        assert_ne!(first, second);
    }

    #[test]
    fn formats_rfc_5322_dates() {
        assert_eq!(
            format_date(UNIX_EPOCH + Duration::from_secs(1_795_385_471)),
            "Sun, 22 Nov 2026 22:11:11 +0000"
        );
    }
}
