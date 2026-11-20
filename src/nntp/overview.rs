use super::{NntpError, NntpResponse};
use crate::persistence::newsrc::ArticleRange;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewFormat {
    fields: Vec<Vec<u8>>,
}

impl OverviewFormat {
    pub fn parse(lines: &[Vec<u8>]) -> Self {
        let fields = lines
            .iter()
            .filter(|line| !line.is_empty())
            .map(|line| Self::overview_field_name(line))
            .collect::<Vec<_>>();
        if fields.is_empty() {
            Self::standard()
        } else {
            Self { fields }
        }
    }

    fn overview_field_name(line: &[u8]) -> Vec<u8> {
        if line.starts_with(b":") {
            return line.to_vec();
        }
        line.iter()
            .position(|byte| *byte == b':')
            .map_or_else(|| line.to_vec(), |index| line[..=index].to_vec())
    }

    pub fn standard() -> Self {
        Self {
            fields: [
                b"Subject:".to_vec(),
                b"From:".to_vec(),
                b"Date:".to_vec(),
                b"Message-ID:".to_vec(),
                b"References:".to_vec(),
                b":bytes".to_vec(),
                b":lines".to_vec(),
            ]
            .to_vec(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewArticle {
    pub article_number: u64,
    pub subject: Option<Vec<u8>>,
    pub from: Option<Vec<u8>>,
    pub date: Option<Vec<u8>>,
    pub message_id: Option<Vec<u8>>,
    pub references: Vec<Vec<u8>>,
    pub invalid_references: Vec<Vec<u8>>,
    pub bytes: Option<u64>,
    pub lines: Option<u64>,
    pub unknown_fields: Vec<(Vec<u8>, Vec<u8>)>,
    pub invalid_fields: Vec<(Vec<u8>, Vec<u8>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidOverviewRowReason {
    ArticleNumber,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidOverviewRow {
    pub line_number: usize,
    pub reason: InvalidOverviewRowReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewRows {
    pub articles: Vec<OverviewArticle>,
    pub invalid_rows: Vec<InvalidOverviewRow>,
    pub out_of_range_rows: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverviewError {
    Transport(NntpError),
    InvalidRange,
    NoSelectedGroup,
    CapabilitiesNotLoaded,
    Unsupported,
    UnexpectedStatus(u16),
}

impl std::fmt::Display for OverviewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => write!(formatter, "{error}"),
            Self::InvalidRange => write!(formatter, "invalid overview range"),
            Self::NoSelectedGroup => write!(formatter, "select a group before loading overview"),
            Self::CapabilitiesNotLoaded => write!(formatter, "server capabilities are not loaded"),
            Self::Unsupported => write!(formatter, "server does not support OVER or XOVER"),
            Self::UnexpectedStatus(status) => {
                write!(formatter, "unexpected overview response status {status}")
            }
        }
    }
}

impl std::error::Error for OverviewError {}

pub fn parse_overview_rows(
    lines: &[Vec<u8>],
    format: &OverviewFormat,
    requested: ArticleRange,
) -> OverviewRows {
    let mut articles = BTreeMap::new();
    let mut invalid_rows = Vec::new();
    let mut out_of_range_rows = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        let mut columns = line.split(|byte| *byte == b'\t');
        let number = columns.next().and_then(parse_number);
        let Some(article_number) = number else {
            invalid_rows.push(InvalidOverviewRow {
                line_number: index + 1,
                reason: InvalidOverviewRowReason::ArticleNumber,
            });
            continue;
        };
        if article_number < requested.first || article_number > requested.last {
            out_of_range_rows.push(index + 1);
            continue;
        }

        let values = columns.collect::<Vec<_>>();
        let mut article = OverviewArticle {
            article_number,
            subject: None,
            from: None,
            date: None,
            message_id: None,
            references: Vec::new(),
            invalid_references: Vec::new(),
            bytes: None,
            lines: None,
            unknown_fields: Vec::new(),
            invalid_fields: Vec::new(),
        };

        for (column_index, value) in values.iter().enumerate() {
            let Some(field) = format.fields.get(column_index) else {
                article.unknown_fields.push((
                    format!("unnamed-{column_index}").into_bytes(),
                    value.to_vec(),
                ));
                continue;
            };
            match canonical_field(field).as_slice() {
                b"subject" => article.subject = Some(value.to_vec()),
                b"from" => article.from = Some(value.to_vec()),
                b"date" => article.date = Some(value.to_vec()),
                b"message-id" => article.message_id = Some(normalize_message_id(value)),
                b"references" => {
                    for reference in value.split(|byte| byte.is_ascii_whitespace()) {
                        if reference.is_empty() {
                            continue;
                        }
                        if valid_message_id(reference) {
                            article.references.push(normalize_message_id(reference));
                        } else {
                            article.invalid_references.push(reference.to_vec());
                        }
                    }
                }
                b"bytes" => match parse_number(value) {
                    Some(bytes) => article.bytes = Some(bytes),
                    None => article.invalid_fields.push((field.clone(), value.to_vec())),
                },
                b"lines" => match parse_number(value) {
                    Some(lines) => article.lines = Some(lines),
                    None => article.invalid_fields.push((field.clone(), value.to_vec())),
                },
                _ => article.unknown_fields.push((field.clone(), value.to_vec())),
            }
        }
        let complete = values.len() >= format.fields.len();
        match articles.get_mut(&article_number) {
            Some((_, previous_complete)) if *previous_complete && !complete => {}
            Some((previous, previous_complete)) => {
                *previous = article;
                *previous_complete = complete;
            }
            None => {
                articles.insert(article_number, (article, complete));
            }
        }
    }

    OverviewRows {
        articles: articles.into_values().map(|(article, _)| article).collect(),
        invalid_rows,
        out_of_range_rows,
    }
}

fn canonical_field(field: &[u8]) -> Vec<u8> {
    let mut name = field;
    if name.starts_with(b":") {
        name = &name[1..];
    }
    if name.ends_with(b":") {
        name = &name[..name.len() - 1];
    }
    if name.ends_with(b":full") {
        name = &name[..name.len() - 5];
    }
    name.iter().map(u8::to_ascii_lowercase).collect()
}

fn normalize_message_id(value: &[u8]) -> Vec<u8> {
    if value.starts_with(b"<") && value.ends_with(b">") && value.len() >= 2 {
        value[1..value.len() - 1].to_vec()
    } else {
        value.to_vec()
    }
}

fn valid_message_id(value: &[u8]) -> bool {
    value.len() > 2
        && value.starts_with(b"<")
        && value.ends_with(b">")
        && !value[1..value.len() - 1]
            .iter()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
}

fn parse_number(value: &[u8]) -> Option<u64> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return None;
    }
    value.iter().try_fold(0u64, |number, digit| {
        number.checked_mul(10)?.checked_add(u64::from(digit - b'0'))
    })
}

pub(super) fn ensure_overview_status(
    response: NntpResponse,
) -> Result<Vec<Vec<u8>>, OverviewError> {
    if response.status != 224 {
        return Err(OverviewError::UnexpectedStatus(response.status));
    }
    Ok(response.data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_named_fields_and_preserves_unknown_fields() {
        let format = OverviewFormat::parse(&[
            b"Subject:".to_vec(),
            b"From:".to_vec(),
            b"Message-ID:".to_vec(),
            b"References:".to_vec(),
            b"X-Custom:".to_vec(),
            b":bytes".to_vec(),
            b":lines".to_vec(),
        ]);
        let rows = parse_overview_rows(
            &[b"10\tHello\tA <a@b>\t<id@b>\t<parent@b> bad\tcustom\t42\t5".to_vec()],
            &format,
            ArticleRange { first: 1, last: 10 },
        );
        assert!(rows.invalid_rows.is_empty());
        assert_eq!(
            rows.articles[0].subject.as_deref(),
            Some(b"Hello".as_slice())
        );
        assert_eq!(
            rows.articles[0].message_id.as_deref(),
            Some(b"id@b".as_slice())
        );
        assert_eq!(rows.articles[0].references, [b"parent@b".to_vec()]);
        assert_eq!(rows.articles[0].invalid_references, [b"bad".to_vec()]);
        assert_eq!(rows.articles[0].unknown_fields[0].1, b"custom");
        assert_eq!(rows.articles[0].bytes, Some(42));
        assert_eq!(rows.articles[0].lines, Some(5));
    }

    #[test]
    fn uses_standard_order_for_unnamed_rows() {
        let format = OverviewFormat::standard();
        let rows = parse_overview_rows(
            &[b"7\tSubject\tFrom\tDate\t<id@host>\t<parent@host>\t99\t4".to_vec()],
            &format,
            ArticleRange { first: 1, last: 10 },
        );
        assert_eq!(rows.articles[0].article_number, 7);
        assert_eq!(
            rows.articles[0].subject.as_deref(),
            Some(b"Subject".as_slice())
        );
        assert_eq!(rows.articles[0].from.as_deref(), Some(b"From".as_slice()));
        assert_eq!(rows.articles[0].bytes, Some(99));
        assert_eq!(rows.articles[0].lines, Some(4));
    }

    #[test]
    fn retains_rows_with_bad_optional_fields_and_last_duplicate_wins() {
        let format = OverviewFormat::standard();
        let rows = parse_overview_rows(
            &[
                b"5\tOld\tFrom\tDate\t<old@host>\t\t12\t3".to_vec(),
                b"5\tNew\tFrom\tDate\t<new@host>\t\tbad\tbad".to_vec(),
            ],
            &format,
            ArticleRange { first: 1, last: 10 },
        );
        assert_eq!(rows.articles.len(), 1);
        assert_eq!(rows.articles[0].subject.as_deref(), Some(b"New".as_slice()));
        assert_eq!(
            rows.articles[0].message_id.as_deref(),
            Some(b"new@host".as_slice())
        );
        assert_eq!(rows.articles[0].bytes, None);
        assert_eq!(rows.articles[0].lines, None);
        assert_eq!(
            rows.articles[0].invalid_fields,
            [
                (b":bytes".to_vec(), b"bad".to_vec()),
                (b":lines".to_vec(), b"bad".to_vec()),
            ]
        );
    }

    #[test]
    fn incomplete_duplicate_does_not_replace_last_complete_row() {
        let format = OverviewFormat::standard();
        let rows = parse_overview_rows(
            &[
                b"5\tComplete\tFrom\tDate\t<id@host>\t\t12\t3".to_vec(),
                b"5\tPartial".to_vec(),
            ],
            &format,
            ArticleRange { first: 1, last: 10 },
        );
        assert_eq!(
            rows.articles[0].subject.as_deref(),
            Some(b"Complete".as_slice())
        );
        assert_eq!(
            rows.articles[0].message_id.as_deref(),
            Some(b"id@host".as_slice())
        );
    }

    #[test]
    fn reports_bad_article_numbers_and_ignores_out_of_range_rows() {
        let rows = parse_overview_rows(
            &[
                b"x\tbad".to_vec(),
                b"11\toutside".to_vec(),
                b"3\tinside".to_vec(),
            ],
            &OverviewFormat::standard(),
            ArticleRange { first: 1, last: 10 },
        );
        assert_eq!(rows.invalid_rows[0].line_number, 1);
        assert_eq!(rows.out_of_range_rows, [2]);
        assert_eq!(rows.articles[0].article_number, 3);
    }

    #[test]
    fn handles_empty_responses_and_optional_missing_columns() {
        assert!(parse_overview_rows(
            &[],
            &OverviewFormat::standard(),
            ArticleRange { first: 1, last: 10 }
        )
        .articles
        .is_empty());
        let rows = parse_overview_rows(
            &[b"2".to_vec()],
            &OverviewFormat::standard(),
            ArticleRange { first: 1, last: 10 },
        );
        assert_eq!(rows.articles[0].article_number, 2);
        assert_eq!(rows.articles[0].subject, None);
    }
}
