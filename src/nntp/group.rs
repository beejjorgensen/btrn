use super::{NntpError, NntpResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupSelection {
    pub name: String,
    pub count: u64,
    pub first_article: u64,
    pub last_article: u64,
    pub count_consistent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupSelectionError {
    Transport(NntpError),
    InvalidName,
    UnknownGroup { response: Vec<u8> },
    UnexpectedStatus { status: u16, response: Vec<u8> },
    MalformedResponse { response: Vec<u8> },
}

impl std::fmt::Display for GroupSelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "{error}"),
            Self::InvalidName => write!(formatter, "invalid newsgroup name"),
            Self::UnknownGroup { .. } => write!(formatter, "unknown or unavailable newsgroup"),
            Self::UnexpectedStatus { status, .. } => {
                write!(formatter, "unexpected GROUP response status {status}")
            }
            Self::MalformedResponse { .. } => {
                write!(formatter, "malformed GROUP response")
            }
        }
    }
}

impl std::error::Error for GroupSelectionError {}

/// Whether `name` can be sent as a single command argument.
pub(super) fn is_valid_group_argument(name: &str) -> bool {
    !name.is_empty()
        && name.is_ascii()
        && !name
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
}

pub(super) fn parse_group_response(
    response: NntpResponse,
    requested_name: &str,
) -> Result<GroupSelection, GroupSelectionError> {
    if response.status == 411 {
        return Err(GroupSelectionError::UnknownGroup {
            response: response.first_line,
        });
    }
    if response.status != 211 {
        return Err(GroupSelectionError::UnexpectedStatus {
            status: response.status,
            response: response.first_line,
        });
    }

    let fields: Vec<&[u8]> = response
        .first_line
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty())
        .collect();
    let [status, count, first, last, name] = fields.as_slice() else {
        return Err(GroupSelectionError::MalformedResponse {
            response: response.first_line,
        });
    };

    let parsed = (
        parse_number(count),
        parse_number(first),
        parse_number(last),
        std::str::from_utf8(name),
    );
    let (Some(count), Some(first_article), Some(last_article), Ok(name)) = parsed else {
        return Err(GroupSelectionError::MalformedResponse {
            response: response.first_line,
        });
    };
    if *status != b"211" || !name.is_ascii() || name != requested_name {
        return Err(GroupSelectionError::MalformedResponse {
            response: response.first_line,
        });
    }

    let reported_range_count = match (first_article, last_article) {
        (0, 0) => Some(0),
        (first, last) if first > 0 && last >= first => last
            .checked_sub(first)
            .and_then(|range| range.checked_add(1)),
        _ => None,
    };

    Ok(GroupSelection {
        name: name.to_owned(),
        count,
        first_article,
        last_article,
        count_consistent: reported_range_count == Some(count),
    })
}

fn parse_number(value: &[u8]) -> Option<u64> {
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

    fn response(status: u16, first_line: &[u8]) -> NntpResponse {
        NntpResponse {
            status,
            first_line: first_line.to_vec(),
            data: Vec::new(),
        }
    }

    #[test]
    fn parses_successful_group_selection() {
        let selection =
            parse_group_response(response(211, b"211 10 101 110 comp.test"), "comp.test").unwrap();
        assert_eq!(
            selection,
            GroupSelection {
                name: "comp.test".into(),
                count: 10,
                first_article: 101,
                last_article: 110,
                count_consistent: true,
            }
        );
    }

    #[test]
    fn preserves_range_when_count_is_inconsistent() {
        let selection =
            parse_group_response(response(211, b"211 3 101 110 comp.test"), "comp.test").unwrap();
        assert_eq!(selection.count, 3);
        assert_eq!(selection.first_article, 101);
        assert_eq!(selection.last_article, 110);
        assert!(!selection.count_consistent);
    }

    #[test]
    fn accepts_empty_group_range() {
        let selection =
            parse_group_response(response(211, b"211 0 0 0 comp.test"), "comp.test").unwrap();
        assert_eq!(selection.count, 0);
        assert_eq!(selection.first_article, 0);
        assert_eq!(selection.last_article, 0);
        assert!(selection.count_consistent);
    }

    #[test]
    fn distinguishes_unknown_group_status() {
        assert_eq!(
            parse_group_response(response(411, b"411 no such group"), "comp.test"),
            Err(GroupSelectionError::UnknownGroup {
                response: b"411 no such group".to_vec(),
            })
        );
    }

    #[test]
    fn rejects_malformed_success_response() {
        for line in [
            b"211 1 1 group.test".as_slice(),
            b"211 x 1 1 comp.test",
            b"211 1 1 1 wrong.test",
            b"211 18446744073709551616 1 1 comp.test",
        ] {
            assert!(matches!(
                parse_group_response(response(211, line), "comp.test"),
                Err(GroupSelectionError::MalformedResponse { .. })
            ));
        }
    }
}
