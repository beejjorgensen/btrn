//! Followup posting: composing a draft from the article being answered,
//! parsing and checking the user's edited composition, and building the
//! article sent with `POST`. Everything here except `compose_file` is pure
//! and does no terminal or network I/O.

pub mod compose_file;
pub mod composition;
pub mod draft;
pub mod mailbox;
pub mod outgoing;

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The poster's identity from the configuration and environment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PostingIdentity {
    pub display_name: Option<String>,
    pub display_email: Option<String>,
    pub organization: Option<String>,
}

/// Configuration the posting flow needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostingSettings {
    pub identity: PostingIdentity,
    /// Editor program followed by its arguments.
    pub editor: Vec<String>,
}

/// A value that is unset when missing, empty, or only whitespace.
pub(crate) fn nonblank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// A hard-to-guess hexadecimal token for file names and message IDs. The
/// standard library's `RandomState` is seeded from the operating system.
pub(crate) fn random_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut token = String::new();
    for round in 0..2u64 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        hasher.write_u64(round);
        hasher.write_u32(std::process::id());
        hasher.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos()),
        );
        token.push_str(&format!("{:016x}", hasher.finish()));
    }
    token
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_tokens_are_distinct_hex() {
        let first = random_token();
        let second = random_token();
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    /// An encoded-word in the followed-up-on article decodes to line breaks;
    /// they must not turn into headers of the followup.
    #[test]
    fn remote_encoded_line_breaks_never_become_posted_headers() {
        use crate::model::article::RawArticle;
        use base64::{engine::general_purpose::STANDARD, Engine};

        let subject = STANDARD.encode("hello\nControl: cancel <victim@example.com>");
        let raw = [
            "From: Poster <poster@example.com>".to_owned(),
            "Newsgroups: comp.test".to_owned(),
            format!("Subject: =?UTF-8?B?{subject}?="),
            "Message-ID: <orig@example.com>".to_owned(),
            String::new(),
            "body".to_owned(),
        ]
        .map(String::into_bytes);
        let decoded = crate::decoder::decode_article(&RawArticle::parse(&raw).article);
        let identity = PostingIdentity {
            display_name: None,
            display_email: Some("me@example.com".to_owned()),
            organization: None,
        };
        let draft = draft::compose_followup(&decoded, &identity, draft::Quote::Original);
        let parsed = composition::parse_composition(draft.text.as_bytes());
        assert_eq!(parsed.errors, []);
        let posted = outgoing::build_article(
            &parsed.composition,
            &outgoing::OutgoingContext {
                references: &draft.references,
                message_id: "<new@example.com>",
                date: "d",
            },
        );
        let headers = posted
            .iter()
            .take_while(|line| !line.is_empty())
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .collect::<Vec<_>>();
        assert!(
            headers.contains(&"Subject: Re: hello Control: cancel <victim@example.com>".to_owned())
        );
        assert!(!headers.iter().any(|line| line.starts_with("Control:")));
    }

    /// The posted headers of a draft after a user types `typed` below it.
    fn posted_headers(draft: &draft::Draft, typed: &str) -> Vec<String> {
        let parsed = composition::parse_composition(format!("{}{typed}", draft.text).as_bytes());
        assert_eq!(parsed.errors, []);
        outgoing::build_article(
            &parsed.composition,
            &outgoing::OutgoingContext {
                references: &draft.references,
                message_id: "<new@example.com>",
                date: "d",
            },
        )
        .iter()
        .take_while(|line| !line.is_empty())
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .collect()
    }

    #[test]
    fn unquoted_followups_and_new_threads_post_the_right_threading_headers() {
        use crate::decoder::{DecodedArticle, DecodedHeader};
        let parent = DecodedArticle {
            headers: [
                ("Newsgroups", "comp.test,comp.misc"),
                ("Subject", "topic"),
                ("Message-ID", "<orig@example.com>"),
            ]
            .map(|(name, value)| DecodedHeader {
                name: name.to_owned(),
                value: value.to_owned(),
            })
            .to_vec(),
            body: "parent text\n".to_owned(),
            issues: Vec::new(),
        };
        let identity = PostingIdentity {
            display_name: None,
            display_email: Some("me@example.com".to_owned()),
            organization: None,
        };
        let header_lines = |headers: Vec<String>| {
            headers
                .into_iter()
                .filter(|line| {
                    ["Newsgroups:", "Subject:", "References:", "In-Reply-To:"]
                        .iter()
                        .any(|name| line.starts_with(name))
                })
                .collect::<Vec<_>>()
        };

        let quoted = draft::compose_followup(&parent, &identity, draft::Quote::Original);
        let unquoted = draft::compose_followup(&parent, &identity, draft::Quote::Nothing);
        assert!(!unquoted.text.contains("parent text"));
        let expected = [
            "Newsgroups: comp.test,comp.misc",
            "Subject: Re: topic",
            "References: <orig@example.com>",
        ];
        assert_eq!(header_lines(posted_headers(&quoted, "reply\n")), expected);
        assert_eq!(header_lines(posted_headers(&unquoted, "reply\n")), expected);

        let new_thread = draft::compose_new_thread("comp.test", "fresh", &identity);
        assert_eq!(
            header_lines(posted_headers(&new_thread, "hello\n")),
            ["Newsgroups: comp.test", "Subject: fresh"]
        );
    }

    #[test]
    fn blank_values_are_unset() {
        assert_eq!(nonblank(Some("  ")), None);
        assert_eq!(nonblank(None), None);
        assert_eq!(nonblank(Some(" a ")), Some("a"));
    }
}
