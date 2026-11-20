#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderField {
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawArticle {
    pub headers: Vec<HeaderField>,
    pub body: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleParseIssue {
    MissingHeaderBodySeparator,
    InvalidHeader { line_number: usize, line: Vec<u8> },
    OrphanContinuation { line_number: usize, line: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleParseResult {
    pub article: RawArticle,
    pub issues: Vec<ArticleParseIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageId(Vec<u8>);

impl MessageId {
    pub fn parse(value: &[u8]) -> Option<Self> {
        if value.len() <= 2 || !value.starts_with(b"<") || !value.ends_with(b">") {
            return None;
        }
        let inner = &value[1..value.len() - 1];
        if inner.is_empty()
            || inner
                .iter()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control() || *byte == b'<')
        {
            return None;
        }
        Some(Self(inner.to_vec()))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleMetadata {
    pub message_id: Option<MessageId>,
    pub references: Vec<MessageId>,
    pub in_reply_to: Option<MessageId>,
    pub from: Option<Vec<u8>>,
    pub subject: Option<Vec<u8>>,
    pub date: Option<Vec<u8>>,
    pub issues: Vec<ArticleMetadataIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleMetadataIssue {
    InvalidMessageId { field: &'static str, value: Vec<u8> },
    InvalidReference { field: &'static str, value: Vec<u8> },
}

impl RawArticle {
    pub fn parse(lines: &[Vec<u8>]) -> ArticleParseResult {
        let mut headers: Vec<HeaderField> = Vec::new();
        let mut body = Vec::new();
        let mut issues = Vec::new();
        let mut in_body = false;
        let mut can_continue = false;

        for (index, line) in lines.iter().enumerate() {
            if in_body {
                body.push(line.clone());
                continue;
            }
            if line.is_empty() {
                in_body = true;
                can_continue = false;
                continue;
            }
            if matches!(line.first(), Some(b' ' | b'\t')) {
                if can_continue {
                    if let Some(header) = headers.last_mut() {
                        header.value.push(b' ');
                        header
                            .value
                            .extend_from_slice(trim_leading_whitespace(line));
                    }
                } else {
                    issues.push(ArticleParseIssue::OrphanContinuation {
                        line_number: index + 1,
                        line: line.clone(),
                    });
                }
                continue;
            }

            let Some(separator) = line.iter().position(|byte| *byte == b':') else {
                issues.push(ArticleParseIssue::InvalidHeader {
                    line_number: index + 1,
                    line: line.clone(),
                });
                can_continue = false;
                continue;
            };
            let name = &line[..separator];
            if !valid_field_name(name) {
                issues.push(ArticleParseIssue::InvalidHeader {
                    line_number: index + 1,
                    line: line.clone(),
                });
                can_continue = false;
                continue;
            }
            headers.push(HeaderField {
                name: name.to_vec(),
                value: trim_leading_whitespace(&line[separator + 1..]).to_vec(),
            });
            can_continue = true;
        }

        if !in_body {
            issues.push(ArticleParseIssue::MissingHeaderBodySeparator);
        }
        ArticleParseResult {
            article: RawArticle { headers, body },
            issues,
        }
    }

    pub fn header(&self, name: &[u8]) -> Option<&[u8]> {
        self.headers
            .iter()
            .rev()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.as_slice())
    }

    pub fn metadata(&self) -> ArticleMetadata {
        let mut issues = Vec::new();
        let message_id = self
            .header(b"Message-ID")
            .and_then(|value| parse_single_message_id("Message-ID", value, &mut issues));
        let references = self
            .header(b"References")
            .map(|value| parse_message_id_list("References", value, &mut issues))
            .unwrap_or_default();
        let in_reply_to = self
            .header(b"In-Reply-To")
            .and_then(|value| parse_single_message_id("In-Reply-To", value, &mut issues));
        ArticleMetadata {
            message_id,
            references,
            in_reply_to,
            from: self.header(b"From").map(<[u8]>::to_vec),
            subject: self.header(b"Subject").map(<[u8]>::to_vec),
            date: self.header(b"Date").map(<[u8]>::to_vec),
            issues,
        }
    }
}

fn parse_single_message_id(
    field: &'static str,
    value: &[u8],
    issues: &mut Vec<ArticleMetadataIssue>,
) -> Option<MessageId> {
    let tokens = value
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if let [token] = tokens.as_slice() {
        if let Some(message_id) = MessageId::parse(token) {
            return Some(message_id);
        }
    }
    issues.push(ArticleMetadataIssue::InvalidMessageId {
        field,
        value: value.to_vec(),
    });
    None
}

fn parse_message_id_list(
    field: &'static str,
    value: &[u8],
    issues: &mut Vec<ArticleMetadataIssue>,
) -> Vec<MessageId> {
    value
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|token| !token.is_empty())
        .filter_map(|token| match MessageId::parse(token) {
            Some(message_id) => Some(message_id),
            None => {
                issues.push(ArticleMetadataIssue::InvalidReference {
                    field,
                    value: token.to_vec(),
                });
                None
            }
        })
        .collect()
}

fn trim_leading_whitespace(value: &[u8]) -> &[u8] {
    let first_non_whitespace = value
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(value.len());
    &value[first_non_whitespace..]
}

fn valid_field_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.iter().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_headers_from_body_and_unfolds_continuations() {
        let parsed = RawArticle::parse(&[
            b"Subject:  hello".to_vec(),
            b"\tcontinued".to_vec(),
            b"X-Raw: \xff".to_vec(),
            b"".to_vec(),
            b"body line".to_vec(),
            b"..dot-stuffed line".to_vec(),
        ]);
        assert!(parsed.issues.is_empty());
        assert_eq!(
            parsed.article.headers,
            [
                HeaderField {
                    name: b"Subject".to_vec(),
                    value: b"hello continued".to_vec(),
                },
                HeaderField {
                    name: b"X-Raw".to_vec(),
                    value: b"\xff".to_vec(),
                },
            ]
        );
        assert_eq!(
            parsed.article.body,
            [b"body line".to_vec(), b"..dot-stuffed line".to_vec()]
        );
        assert_eq!(
            parsed.article.header(b"subject"),
            Some(b"hello continued".as_slice())
        );
    }

    #[test]
    fn keeps_duplicate_headers_and_uses_last_value_case_insensitively() {
        let parsed = RawArticle::parse(&[
            b"Subject: first".to_vec(),
            b"subject: second".to_vec(),
            b"".to_vec(),
        ]);
        assert!(parsed.issues.is_empty());
        assert_eq!(parsed.article.headers.len(), 2);
        assert_eq!(
            parsed.article.header(b"SUBJECT"),
            Some(b"second".as_slice())
        );
    }

    #[test]
    fn reports_orphan_continuations_and_malformed_headers() {
        let parsed = RawArticle::parse(&[
            b" orphan".to_vec(),
            b"Subject: valid".to_vec(),
            b"bad header".to_vec(),
            b"\tcontinuation after malformed header".to_vec(),
            b"Bad Name: invalid field name".to_vec(),
            b"".to_vec(),
        ]);
        assert_eq!(
            parsed.issues,
            [
                ArticleParseIssue::OrphanContinuation {
                    line_number: 1,
                    line: b" orphan".to_vec(),
                },
                ArticleParseIssue::InvalidHeader {
                    line_number: 3,
                    line: b"bad header".to_vec(),
                },
                ArticleParseIssue::OrphanContinuation {
                    line_number: 4,
                    line: b"\tcontinuation after malformed header".to_vec(),
                },
                ArticleParseIssue::InvalidHeader {
                    line_number: 5,
                    line: b"Bad Name: invalid field name".to_vec(),
                },
            ]
        );
        assert_eq!(parsed.article.headers.len(), 1);
    }

    #[test]
    fn reports_missing_separator_without_treating_headers_as_body() {
        let parsed = RawArticle::parse(&[b"Subject: no body separator".to_vec()]);
        assert_eq!(
            parsed.issues,
            [ArticleParseIssue::MissingHeaderBodySeparator]
        );
        assert_eq!(parsed.article.headers.len(), 1);
        assert!(parsed.article.body.is_empty());
    }

    #[test]
    fn extracts_typed_metadata_with_normalized_message_ids() {
        let parsed = RawArticle::parse(&[
            b"Message-ID: <article@example>".to_vec(),
            b"References: <root@example> <parent@example>".to_vec(),
            b"In-Reply-To: <parent@example>".to_vec(),
            b"From: \xff".to_vec(),
            b"Subject: subject".to_vec(),
            b"Date: today".to_vec(),
            b"".to_vec(),
        ]);
        let metadata = parsed.article.metadata();
        assert_eq!(
            metadata.message_id,
            Some(MessageId(b"article@example".to_vec()))
        );
        assert_eq!(
            metadata.references,
            [
                MessageId(b"root@example".to_vec()),
                MessageId(b"parent@example".to_vec()),
            ]
        );
        assert_eq!(
            metadata.in_reply_to,
            Some(MessageId(b"parent@example".to_vec()))
        );
        assert_eq!(metadata.from, Some(b"\xff".to_vec()));
        assert_eq!(metadata.subject, Some(b"subject".to_vec()));
        assert_eq!(metadata.date, Some(b"today".to_vec()));
        assert!(metadata.issues.is_empty());
    }

    #[test]
    fn reports_malformed_message_ids_and_keeps_valid_references() {
        let parsed = RawArticle::parse(&[
            b"Message-ID: not-an-id".to_vec(),
            b"References: <valid@example> broken <bad id>".to_vec(),
            b"In-Reply-To: <first@example> <second@example>".to_vec(),
            b"".to_vec(),
        ]);
        let metadata = parsed.article.metadata();
        assert_eq!(metadata.message_id, None);
        assert_eq!(metadata.references, [MessageId(b"valid@example".to_vec())]);
        assert_eq!(metadata.in_reply_to, None);
        assert_eq!(
            metadata.issues,
            [
                ArticleMetadataIssue::InvalidMessageId {
                    field: "Message-ID",
                    value: b"not-an-id".to_vec(),
                },
                ArticleMetadataIssue::InvalidReference {
                    field: "References",
                    value: b"broken".to_vec(),
                },
                ArticleMetadataIssue::InvalidReference {
                    field: "References",
                    value: b"<bad".to_vec(),
                },
                ArticleMetadataIssue::InvalidReference {
                    field: "References",
                    value: b"id>".to_vec(),
                },
                ArticleMetadataIssue::InvalidMessageId {
                    field: "In-Reply-To",
                    value: b"<first@example> <second@example>".to_vec(),
                },
            ]
        );
    }

    #[test]
    fn metadata_uses_last_duplicate_field_and_preserves_bytes() {
        let parsed = RawArticle::parse(&[
            b"Message-ID: <old@example>".to_vec(),
            b"Subject: old".to_vec(),
            b"message-id: <new@example>".to_vec(),
            b"subject: \xff".to_vec(),
            b"".to_vec(),
        ]);
        let metadata = parsed.article.metadata();
        assert_eq!(
            metadata.message_id,
            Some(MessageId(b"new@example".to_vec()))
        );
        assert_eq!(metadata.subject, Some(b"\xff".to_vec()));
    }
}
