//! Builds the composition file's initial text for a followup.

use super::{nonblank, PostingIdentity};
use crate::decoder::DecodedArticle;
use crate::model::article::{parse_message_ids, MessageId};

const ATTRIBUTION_LIMIT: usize = 72;

/// The initial composition text and the facts about the followed-up-on
/// article that are needed again when posting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowupDraft {
    pub text: String,
    /// The `References` for the followup: the original's references
    /// followed by its message ID, or empty when it has no valid message ID.
    pub references: Vec<MessageId>,
    /// The original article asked for replies by email.
    pub followup_to_poster: bool,
}

pub fn compose_followup(article: &DecodedArticle, identity: &PostingIdentity) -> FollowupDraft {
    let header = |name: &str| {
        nonblank(
            article
                .headers
                .iter()
                .find(|header| header.name.eq_ignore_ascii_case(name))
                .map(|header| header.value.as_str()),
        )
    };

    let followup_to = header("Followup-To");
    let followup_to_poster = followup_to.is_some_and(|value| value.eq_ignore_ascii_case("poster"));
    let newsgroups = match followup_to {
        Some(groups) if !followup_to_poster => groups,
        _ => header("Newsgroups").unwrap_or(""),
    };

    let subject = match header("Subject") {
        Some(subject) if starts_with_reply_prefix(subject) => subject.to_owned(),
        Some(subject) => format!("Re: {subject}"),
        None => "Re: [No subject]".to_owned(),
    };

    let message_id = header("Message-ID").and_then(|value| MessageId::parse(value.as_bytes()));
    // Without the original's own Message-ID the chain cannot be continued,
    // so References is omitted entirely.
    let references = match &message_id {
        Some(message_id) => {
            let mut references = header("References")
                .map(|value| parse_message_ids(value.as_bytes()))
                .unwrap_or_default();
            references.push(message_id.clone());
            references
        }
        None => Vec::new(),
    };

    let mut lines = vec![
        header_line("Newsgroups", newsgroups),
        header_line("Subject", &subject),
        header_line("Followup-To", ""),
        header_line("From", &from_value(identity)),
        header_line(
            "Organization",
            nonblank(identity.organization.as_deref()).unwrap_or(""),
        ),
        header_line("Keywords", ""),
        String::new(),
    ];
    lines.extend(attribution_lines(
        message_id.as_ref(),
        header("From").unwrap_or("Anonymous"),
    ));
    lines.extend(quote_body(&article.body));

    let mut text = lines.join("\n");
    text.push('\n');
    FollowupDraft {
        text,
        references,
        followup_to_poster,
    }
}

/// A draft header line. Values can come from the followed-up-on article, and
/// decoding can produce line breaks, so every control character becomes a
/// space; otherwise a remote `Subject` could add header lines to the draft.
fn header_line(name: &str, value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let value = value.trim();
    if value.is_empty() {
        format!("{name}:")
    } else {
        format!("{name}: {value}")
    }
}

fn starts_with_reply_prefix(subject: &str) -> bool {
    subject
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("re:"))
}

/// The `From` line from the configured identity; missing parts are left out.
fn from_value(identity: &PostingIdentity) -> String {
    match (
        nonblank(identity.display_name.as_deref()),
        nonblank(identity.display_email.as_deref()),
    ) {
        (Some(name), Some(email)) => format!("{name} <{email}>"),
        (None, Some(email)) => email.to_owned(),
        (Some(name), None) => name.to_owned(),
        (None, None) => String::new(),
    }
}

/// `In article <id>, Poster wrote:`, split after the comma when longer than
/// 72 characters. Without a message ID it is just `Poster wrote:`.
fn attribution_lines(message_id: Option<&MessageId>, poster: &str) -> Vec<String> {
    let Some(message_id) = message_id else {
        return vec![format!("{poster} wrote:")];
    };
    let lead = format!(
        "In article <{}>,",
        String::from_utf8_lossy(message_id.as_bytes())
    );
    let rest = format!("{poster} wrote:");
    let line = format!("{lead} {rest}");
    if line.chars().count() > ATTRIBUTION_LIMIT {
        vec![lead, rest]
    } else {
        vec![line]
    }
}

/// Prefixes every body line with `>` after trimming trailing blank lines.
/// An empty body becomes a single `>`.
fn quote_body(body: &str) -> Vec<String> {
    let mut lines = body
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect::<Vec<_>>();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return vec![">".to_owned()];
    }
    lines.iter().map(|line| format!(">{line}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::DecodedHeader;

    fn article(headers: &[(&str, &str)], body: &str) -> DecodedArticle {
        DecodedArticle {
            headers: headers
                .iter()
                .map(|(name, value)| DecodedHeader {
                    name: (*name).to_owned(),
                    value: (*value).to_owned(),
                })
                .collect(),
            body: body.to_owned(),
            issues: Vec::new(),
        }
    }

    fn identity(
        name: Option<&str>,
        email: Option<&str>,
        organization: Option<&str>,
    ) -> PostingIdentity {
        PostingIdentity {
            display_name: name.map(str::to_owned),
            display_email: email.map(str::to_owned),
            organization: organization.map(str::to_owned),
        }
    }

    fn full_identity() -> PostingIdentity {
        identity(
            Some("Example Name"),
            Some("user@example.com"),
            Some("Yoyodyne Industries"),
        )
    }

    #[test]
    fn remote_line_breaks_cannot_add_header_lines() {
        let draft = compose_followup(
            &article(
                &[
                    ("Newsgroups", "comp.test"),
                    (
                        "Followup-To",
                        "comp.test\nControl: rmgroup comp.test",
                    ),
                    (
                        "Subject",
                        "hello\nControl: cancel <victim@example.com>\r\nApproved: m@example.com\n\tfolded\u{0}",
                    ),
                ],
                "body\n",
            ),
            &full_identity(),
        );
        let header_block = draft.text.split("\n\n").next().unwrap();
        assert_eq!(
            header_block.lines().collect::<Vec<_>>(),
            [
                "Newsgroups: comp.test Control: rmgroup comp.test",
                "Subject: Re: hello Control: cancel <victim@example.com>  Approved: m@example.com  folded",
                "Followup-To:",
                "From: Example Name <user@example.com>",
                "Organization: Yoyodyne Industries",
                "Keywords:",
            ]
        );
    }

    #[test]
    fn composes_the_specified_template() {
        let draft = compose_followup(
            &article(
                &[
                    ("Newsgroups", "comp.lang.c,comp.misc"),
                    ("Subject", "Previous Subject"),
                    ("From", "Poster <poster@example.com>"),
                    ("Message-ID", "<abc@example.org>"),
                    ("References", "<root@example.org> <mid@example.org>"),
                ],
                "first\n> already quoted\n\nlast\n\n\n",
            ),
            &full_identity(),
        );
        assert_eq!(
            draft.text,
            "Newsgroups: comp.lang.c,comp.misc\n\
             Subject: Re: Previous Subject\n\
             Followup-To:\n\
             From: Example Name <user@example.com>\n\
             Organization: Yoyodyne Industries\n\
             Keywords:\n\
             \n\
             In article <abc@example.org>, Poster <poster@example.com> wrote:\n\
             >first\n\
             >> already quoted\n\
             >\n\
             >last\n"
        );
        assert_eq!(
            draft.references,
            [
                MessageId::parse(b"<root@example.org>").unwrap(),
                MessageId::parse(b"<mid@example.org>").unwrap(),
                MessageId::parse(b"<abc@example.org>").unwrap(),
            ]
        );
        assert!(!draft.followup_to_poster);
    }

    fn template_line(draft: &FollowupDraft, name: &str) -> String {
        draft
            .text
            .lines()
            .find(|line| line.starts_with(&format!("{name}:")))
            .unwrap()
            .to_owned()
    }

    #[test]
    fn references_are_omitted_when_the_original_has_no_valid_message_id() {
        let references = |headers: &[(&str, &str)]| {
            compose_followup(&article(headers, "q"), &full_identity()).references
        };
        let parent = ("References", "<root@example> <parent@example>");
        assert!(references(&[parent]).is_empty());
        assert!(references(&[parent, ("Message-ID", "not-an-id")]).is_empty());
        assert_eq!(
            references(&[parent, ("Message-ID", "<self@example>")]),
            ["<root@example>", "<parent@example>", "<self@example>"]
                .map(|id| MessageId::parse(id.as_bytes()).unwrap())
        );
    }

    #[test]
    fn newsgroups_come_from_followup_to_unless_it_is_poster() {
        let identity = full_identity();
        let draft = compose_followup(
            &article(&[("Newsgroups", "a.b,c.d"), ("Followup-To", "c.d")], "x"),
            &identity,
        );
        assert_eq!(template_line(&draft, "Newsgroups"), "Newsgroups: c.d");

        let draft = compose_followup(
            &article(&[("Newsgroups", "a.b"), ("Followup-To", " Poster ")], "x"),
            &identity,
        );
        assert_eq!(template_line(&draft, "Newsgroups"), "Newsgroups: a.b");
        assert!(draft.followup_to_poster);
        assert_eq!(template_line(&draft, "Followup-To"), "Followup-To:");

        let draft = compose_followup(&article(&[], "x"), &identity);
        assert_eq!(template_line(&draft, "Newsgroups"), "Newsgroups:");
    }

    #[test]
    fn subject_adds_re_only_without_an_existing_re_colon_prefix() {
        let subject = |value: Option<&str>| {
            let headers = value
                .map(|value| vec![("Subject", value)])
                .unwrap_or_default();
            template_line(
                &compose_followup(&article(&headers, ""), &full_identity()),
                "Subject",
            )
        };
        assert_eq!(subject(Some("topic")), "Subject: Re: topic");
        assert_eq!(subject(Some("RE: topic")), "Subject: RE: topic");
        assert_eq!(subject(Some("re:topic")), "Subject: re:topic");
        assert_eq!(subject(Some("Re topic")), "Subject: Re: Re topic");
        assert_eq!(subject(Some("Réponse")), "Subject: Re: Réponse");
        assert_eq!(subject(None), "Subject: Re: [No subject]");
        assert_eq!(subject(Some("   ")), "Subject: Re: [No subject]");
    }

    #[test]
    fn from_and_organization_follow_the_configured_identity() {
        let from = |identity: PostingIdentity| {
            template_line(&compose_followup(&article(&[], ""), &identity), "From")
        };
        assert_eq!(
            from(full_identity()),
            "From: Example Name <user@example.com>"
        );
        assert_eq!(
            from(identity(None, Some("user@example.com"), None)),
            "From: user@example.com"
        );
        assert_eq!(
            from(identity(Some("Example Name"), Some("  "), None)),
            "From: Example Name"
        );
        assert_eq!(from(identity(None, None, None)), "From:");
        assert_eq!(
            template_line(
                &compose_followup(&article(&[], ""), &identity(None, None, None)),
                "Organization"
            ),
            "Organization:"
        );
    }

    #[test]
    fn attribution_handles_missing_parts_and_splits_long_lines() {
        let attribution = |headers: &[(&str, &str)]| {
            compose_followup(&article(headers, "q"), &full_identity())
                .text
                .split("\n\n")
                .nth(1)
                .unwrap()
                .lines()
                .take_while(|line| !line.starts_with('>'))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(attribution(&[("From", "P <p@x>")]), ["P <p@x> wrote:"]);
        assert_eq!(
            attribution(&[("Message-ID", "<id@x>")]),
            ["In article <id@x>, Anonymous wrote:"]
        );
        assert_eq!(attribution(&[]), ["Anonymous wrote:"]);
        assert_eq!(
            attribution(&[
                (
                    "Message-ID",
                    "<a-rather-long-message-id.12345@news.example.org>"
                ),
                ("From", "Somebody Long <somebody@example.com>"),
            ]),
            [
                "In article <a-rather-long-message-id.12345@news.example.org>,",
                "Somebody Long <somebody@example.com> wrote:"
            ]
        );
        // Exactly 72 characters stays on one line.
        let poster = "P".repeat(72 - "In article <i@x>,  wrote:".len());
        assert_eq!(
            attribution(&[("Message-ID", "<i@x>"), ("From", &poster)]),
            [format!("In article <i@x>, {poster} wrote:")]
        );
    }

    #[test]
    fn empty_body_quotes_as_a_single_marker() {
        let draft = compose_followup(&article(&[("From", "P")], "\n\n"), &full_identity());
        assert!(draft.text.ends_with("\nP wrote:\n>\n"));
        let draft = compose_followup(&article(&[("From", "P")], ""), &full_identity());
        assert!(draft.text.ends_with("\nP wrote:\n>\n"));
    }

    #[test]
    fn quoting_keeps_signatures_and_strips_carriage_returns() {
        let draft = compose_followup(
            &article(&[("From", "P")], "text\r\n-- \r\nsig\r\n"),
            &full_identity(),
        );
        assert!(draft.text.ends_with("P wrote:\n>text\n>-- \n>sig\n"));
    }
}
