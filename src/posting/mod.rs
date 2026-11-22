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
        let draft = draft::compose_followup(&decoded, &identity);
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

    #[test]
    fn blank_values_are_unset() {
        assert_eq!(nonblank(Some("  ")), None);
        assert_eq!(nonblank(None), None);
        assert_eq!(nonblank(Some(" a ")), Some("a"));
    }
}
