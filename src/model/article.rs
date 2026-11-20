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
}
