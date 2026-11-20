#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostingStatus {
    Allowed,
    Moderated,
    Prohibited,
    Unknown(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveGroup {
    pub name: String,
    pub first_article: u64,
    pub last_article: u64,
    pub posting_status: PostingStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidActiveLineReason {
    FieldCount,
    GroupName,
    ArticleNumber,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidActiveLine {
    pub line_number: usize,
    pub reason: InvalidActiveLineReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveGroupList {
    pub groups: Vec<ActiveGroup>,
    pub invalid_lines: Vec<InvalidActiveLine>,
}

pub fn parse_active_groups(lines: &[Vec<u8>]) -> ActiveGroupList {
    let mut groups = Vec::new();
    let mut invalid_lines = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let fields: Vec<&[u8]> = line
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty())
            .collect();
        let reason = match fields.as_slice() {
            [name, last, first, status] => {
                if !name.is_ascii() {
                    Some(InvalidActiveLineReason::GroupName)
                } else {
                    match (
                        parse_article_number(first),
                        parse_article_number(last),
                        std::str::from_utf8(name),
                    ) {
                        (Some(first_article), Some(last_article), Ok(name)) => {
                            groups.push(ActiveGroup {
                                name: name.to_owned(),
                                first_article,
                                last_article,
                                posting_status: match *status {
                                    b"y" => PostingStatus::Allowed,
                                    b"m" => PostingStatus::Moderated,
                                    b"n" => PostingStatus::Prohibited,
                                    unknown => PostingStatus::Unknown(unknown.to_vec()),
                                },
                            });
                            None
                        }
                        (None, _, _) | (_, None, _) => Some(InvalidActiveLineReason::ArticleNumber),
                        (_, _, Err(_)) => Some(InvalidActiveLineReason::GroupName),
                    }
                }
            }
            _ => Some(InvalidActiveLineReason::FieldCount),
        };

        if let Some(reason) = reason {
            invalid_lines.push(InvalidActiveLine {
                line_number: index + 1,
                reason,
            });
        }
    }

    ActiveGroupList {
        groups,
        invalid_lines,
    }
}

fn parse_article_number(value: &[u8]) -> Option<u64> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return None;
    }
    value.iter().try_fold(0u64, |number, digit| {
        number.checked_mul(10)?.checked_add(u64::from(digit - b'0'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(value: &[u8]) -> Vec<u8> {
        value.to_vec()
    }

    #[test]
    fn parses_group_rows_and_all_known_posting_statuses() {
        let parsed = parse_active_groups(&[
            line(b"comp.lang.rust 900 10 y"),
            line(b"comp.lang.c 0 0 m"),
            line(b"misc.test 100 1 n"),
        ]);

        assert_eq!(
            parsed.groups,
            [
                ActiveGroup {
                    name: "comp.lang.rust".into(),
                    first_article: 10,
                    last_article: 900,
                    posting_status: PostingStatus::Allowed,
                },
                ActiveGroup {
                    name: "comp.lang.c".into(),
                    first_article: 0,
                    last_article: 0,
                    posting_status: PostingStatus::Moderated,
                },
                ActiveGroup {
                    name: "misc.test".into(),
                    first_article: 1,
                    last_article: 100,
                    posting_status: PostingStatus::Prohibited,
                },
            ]
        );
        assert!(parsed.invalid_lines.is_empty());
    }

    #[test]
    fn preserves_unknown_posting_status() {
        let parsed = parse_active_groups(&[line(b"misc.test 20 1 x")]);
        assert_eq!(
            parsed.groups[0].posting_status,
            PostingStatus::Unknown(b"x".to_vec())
        );
        assert!(parsed.invalid_lines.is_empty());
    }

    #[test]
    fn retains_valid_rows_and_reports_malformed_rows() {
        let parsed = parse_active_groups(&[
            line(b"good.group 30 1 y"),
            line(b"bad-number 30x 1 y"),
            line(b"bad-name-\xff 30 1 m"),
            line(b"missing-fields 30 1"),
            line(b"extra fields 30 1 n"),
        ]);

        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(
            parsed.invalid_lines,
            [
                InvalidActiveLine {
                    line_number: 2,
                    reason: InvalidActiveLineReason::ArticleNumber,
                },
                InvalidActiveLine {
                    line_number: 3,
                    reason: InvalidActiveLineReason::GroupName,
                },
                InvalidActiveLine {
                    line_number: 4,
                    reason: InvalidActiveLineReason::FieldCount,
                },
                InvalidActiveLine {
                    line_number: 5,
                    reason: InvalidActiveLineReason::FieldCount,
                },
            ]
        );
    }

    #[test]
    fn rejects_article_number_overflow() {
        let parsed = parse_active_groups(&[line(b"too.large 18446744073709551616 1 y")]);
        assert!(parsed.groups.is_empty());
        assert_eq!(
            parsed.invalid_lines[0].reason,
            InvalidActiveLineReason::ArticleNumber
        );
    }

    #[test]
    fn empty_response_produces_an_empty_list() {
        assert_eq!(
            parse_active_groups(&[]),
            ActiveGroupList {
                groups: Vec::new(),
                invalid_lines: Vec::new(),
            }
        );
    }
}
