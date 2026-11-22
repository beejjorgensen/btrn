use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArticleRange {
    pub first: u64,
    pub last: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsrcEntry {
    pub subscribed: bool,
    pub read_ranges: Vec<ArticleRange>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewsrcState {
    pub entries: BTreeMap<String, NewsrcEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsrcIssue {
    pub line_number: usize,
    pub message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsrcParseResult {
    pub state: NewsrcState,
    pub issues: Vec<NewsrcIssue>,
}

impl NewsrcState {
    pub fn parse(input: &str) -> NewsrcParseResult {
        let mut state = Self::default();
        let mut issues = Vec::new();

        for (index, line) in input.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let Some(delimiter) = line
                .char_indices()
                .find(|(_, character)| *character == ':' || *character == '!')
            else {
                issues.push(NewsrcIssue {
                    line_number: index + 1,
                    message: "missing subscription marker",
                });
                continue;
            };
            let (name, rest) = line.split_at(delimiter.0);
            if name.is_empty() || name.bytes().any(|byte| byte.is_ascii_whitespace()) {
                issues.push(NewsrcIssue {
                    line_number: index + 1,
                    message: "invalid group name",
                });
                continue;
            }
            let marker = delimiter.1;
            let ranges_text = rest[marker.len_utf8()..].trim();
            let mut ranges = Vec::new();
            for token in ranges_text.split(',').map(str::trim) {
                if token.is_empty() {
                    if !ranges_text.is_empty() {
                        issues.push(NewsrcIssue {
                            line_number: index + 1,
                            message: "empty read range",
                        });
                    }
                    continue;
                }
                match parse_range(token) {
                    Some(range) => ranges.push(range),
                    None => issues.push(NewsrcIssue {
                        line_number: index + 1,
                        message: "invalid read range",
                    }),
                }
            }
            state.entries.insert(
                name.to_owned(),
                NewsrcEntry {
                    subscribed: marker == ':',
                    read_ranges: normalize_ranges(ranges),
                },
            );
        }

        NewsrcParseResult { state, issues }
    }

    pub fn serialize(&self) -> String {
        let mut output = String::new();
        for (name, entry) in &self.entries {
            output.push_str(name);
            output.push(if entry.subscribed { ':' } else { '!' });
            output.push(' ');
            let ranges = normalize_ranges(entry.read_ranges.clone());
            for (index, range) in ranges.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&range.first.to_string());
                if range.last != range.first {
                    output.push('-');
                    if range.last != u64::MAX {
                        output.push_str(&range.last.to_string());
                    }
                }
            }
            output.push('\n');
        }
        output
    }

    pub fn write_atomic(&self, path: &Path) -> io::Result<()> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        let file_name = path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "newsrc path has no file name")
        })?;

        let (temporary_path, mut file) = loop {
            let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let mut temporary_name = file_name.to_os_string();
            temporary_name.push(format!(".{}.{}.tmp", std::process::id(), sequence));
            let temporary_path = parent.join(temporary_name);
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)
            {
                Ok(file) => break (temporary_path, file),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        };

        let write_result = (|| {
            file.write_all(self.serialize().as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary_path, path)
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        write_result
    }

    pub fn entry(&self, group: &str) -> Option<&NewsrcEntry> {
        self.entries.get(group)
    }
}

pub fn normalize_ranges(mut ranges: Vec<ArticleRange>) -> Vec<ArticleRange> {
    for range in &mut ranges {
        if range.first > range.last {
            std::mem::swap(&mut range.first, &mut range.last);
        }
    }
    ranges.sort_unstable();
    let mut normalized: Vec<ArticleRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(previous) = normalized.last_mut() {
            if range.first <= previous.last.saturating_add(1) {
                previous.last = previous.last.max(range.last);
                continue;
            }
        }
        normalized.push(range);
    }
    normalized
}

/// Overview chunks covering the `unread_limit` highest-numbered unread
/// articles in the active range `first_article..=last_article`.
pub fn plan_unread_chunks(
    first_article: u64,
    last_article: u64,
    read_ranges: &[ArticleRange],
    unread_limit: usize,
    chunk_size: usize,
) -> Vec<ArticleRange> {
    if chunk_size == 0 {
        return Vec::new();
    }
    chunk_ranges(
        unread_spans(first_article, last_article, read_ranges, unread_limit),
        chunk_size,
    )
}

/// Overview chunks covering the `context_limit` highest-numbered read
/// articles in the active range. Their metadata gives already-read parents
/// and siblings to the thread tree without changing any read state.
pub fn plan_read_context_chunks(
    first_article: u64,
    last_article: u64,
    read_ranges: &[ArticleRange],
    context_limit: usize,
    chunk_size: usize,
) -> Vec<ArticleRange> {
    if chunk_size == 0 {
        return Vec::new();
    }
    chunk_ranges(
        read_context_spans(first_article, last_article, read_ranges, context_limit),
        chunk_size,
    )
}

/// Overview chunks covering both the newest unread articles and the newest
/// read-context articles. Touching spans are joined before chunking, so
/// interleaved read and unread articles need as few requests as one
/// contiguous range.
pub fn plan_overview_chunks(
    first_article: u64,
    last_article: u64,
    read_ranges: &[ArticleRange],
    unread_limit: usize,
    context_limit: usize,
    chunk_size: usize,
) -> Vec<ArticleRange> {
    if chunk_size == 0 {
        return Vec::new();
    }
    let mut spans = unread_spans(first_article, last_article, read_ranges, unread_limit);
    spans.extend(read_context_spans(
        first_article,
        last_article,
        read_ranges,
        context_limit,
    ));
    chunk_ranges(normalize_ranges(spans), chunk_size)
}

/// The `unread_limit` highest-numbered unread articles in the active range,
/// as ascending, disjoint spans.
fn unread_spans(
    first_article: u64,
    last_article: u64,
    read_ranges: &[ArticleRange],
    unread_limit: usize,
) -> Vec<ArticleRange> {
    if first_article == 0 || last_article < first_article || unread_limit == 0 {
        return Vec::new();
    }
    let read = read_within(first_article, last_article, read_ranges);
    let mut spans = Vec::new();
    let mut cursor = Some(first_article);
    for range in read {
        let Some(current) = cursor else {
            break;
        };
        if range.first > current {
            spans.push(ArticleRange {
                first: current,
                last: range.first - 1,
            });
        }
        cursor = range.last.checked_add(1);
    }
    if let Some(current) = cursor {
        if current <= last_article {
            spans.push(ArticleRange {
                first: current,
                last: last_article,
            });
        }
    }
    newest(spans, unread_limit)
}

/// The `context_limit` highest-numbered read articles in the active range,
/// as ascending, disjoint spans.
fn read_context_spans(
    first_article: u64,
    last_article: u64,
    read_ranges: &[ArticleRange],
    context_limit: usize,
) -> Vec<ArticleRange> {
    if first_article == 0 || last_article < first_article || context_limit == 0 {
        return Vec::new();
    }
    newest(
        read_within(first_article, last_article, read_ranges),
        context_limit,
    )
}

/// Normalized read ranges clipped to the active range.
fn read_within(
    first_article: u64,
    last_article: u64,
    read_ranges: &[ArticleRange],
) -> Vec<ArticleRange> {
    normalize_ranges(read_ranges.to_vec())
        .into_iter()
        .filter_map(|range| {
            let first = range.first.max(first_article);
            let last = range.last.min(last_article);
            (first <= last).then_some(ArticleRange { first, last })
        })
        .collect()
}

/// The highest `limit` article numbers from ascending, disjoint `spans`,
/// returned in ascending order.
fn newest(spans: Vec<ArticleRange>, limit: usize) -> Vec<ArticleRange> {
    let mut remaining = limit as u128;
    let mut selected = Vec::new();
    for span in spans.into_iter().rev() {
        if remaining == 0 {
            break;
        }
        let span_len = u128::from(span.last) - u128::from(span.first) + 1;
        let take = remaining.min(span_len);
        let start = span.last - u64::try_from(take - 1).expect("range length fits u64");
        selected.push(ArticleRange {
            first: start,
            last: span.last,
        });
        remaining -= take;
    }
    selected.reverse();
    selected
}

fn chunk_ranges(ranges: Vec<ArticleRange>, chunk_size: usize) -> Vec<ArticleRange> {
    let chunk_size = u64::try_from(chunk_size).unwrap_or(u64::MAX);
    let mut chunks = Vec::new();
    for range in ranges {
        let mut start = range.first;
        loop {
            let end = start
                .saturating_add(chunk_size.saturating_sub(1))
                .min(range.last);
            chunks.push(ArticleRange {
                first: start,
                last: end,
            });
            let Some(next) = end.checked_add(1) else {
                break;
            };
            if next > range.last {
                break;
            }
            start = next;
        }
    }
    chunks
}

fn parse_range(token: &str) -> Option<ArticleRange> {
    let (first, last) = match token.split_once('-') {
        Some(("", last)) => (1, parse_number(last)?),
        Some((first, "")) => (parse_number(first)?, u64::MAX),
        Some((first, last)) => (parse_number(first)?, parse_number(last)?),
        None => {
            let article = parse_number(token)?;
            (article, article)
        }
    };
    Some(ArticleRange { first, last })
}

fn parse_number(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("btrn-newsrc-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn writes_only_newsrc_state_atomically_and_round_trips() {
        let directory = temporary_directory();
        let path = directory.join("nested/newsrc");
        let state = NewsrcState::parse("comp.test: 1-3,8\nmisc.test! 10-\n").state;

        state.write_atomic(&path).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(NewsrcState::parse(&contents).state, state);
        assert!(!contents.contains("Subject"));
        assert!(!contents.contains("Message-ID"));
        assert!(!contents.contains("body"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_replacement_preserves_existing_destination_and_cleans_temporary_file() {
        let directory = temporary_directory();
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("newsrc");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("keep"), b"existing").unwrap();

        let state = NewsrcState::parse("comp.test: 1-3\n").state;
        assert!(state.write_atomic(&destination).is_err());
        assert_eq!(fs::read(destination.join("keep")).unwrap(), b"existing");
        assert_eq!(
            fs::read_dir(&directory).unwrap().count(),
            1,
            "temporary file should be removed after replacement failure"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parses_subscription_markers_ranges_and_issues() {
        let parsed = NewsrcState::parse(
            "comp.test: 1-3,8\n\
             misc.test! 10-8,8,2-\n\
             malformed\n\
             bad.range: 1,nope,4\n",
        );
        assert_eq!(
            parsed.state.entry("comp.test"),
            Some(&NewsrcEntry {
                subscribed: true,
                read_ranges: vec![
                    ArticleRange { first: 1, last: 3 },
                    ArticleRange { first: 8, last: 8 },
                ],
            })
        );
        assert_eq!(
            parsed.state.entry("misc.test"),
            Some(&NewsrcEntry {
                subscribed: false,
                read_ranges: vec![ArticleRange {
                    first: 2,
                    last: u64::MAX
                }],
            })
        );
        assert_eq!(parsed.issues.len(), 2);
    }

    #[test]
    fn serializes_and_round_trips_normalized_state() {
        let parsed = NewsrcState::parse("z.group! 9,1-3,2\na.group: \n");
        let text = parsed.state.serialize();
        assert_eq!(text, "a.group: \nz.group! 1-3,9\n");
        assert_eq!(NewsrcState::parse(&text).state, parsed.state);
    }

    #[test]
    fn reports_empty_range_components_without_dropping_valid_ranges() {
        let parsed = NewsrcState::parse("comp.test: 1,,3,");
        assert_eq!(
            parsed.state.entry("comp.test").unwrap().read_ranges,
            [
                ArticleRange { first: 1, last: 1 },
                ArticleRange { first: 3, last: 3 },
            ]
        );
        assert_eq!(parsed.issues.len(), 2);
    }

    #[test]
    fn plans_newest_unread_chunks_without_treating_gaps_as_read() {
        let chunks = plan_unread_chunks(
            1,
            15,
            &[
                ArticleRange { first: 1, last: 5 },
                ArticleRange {
                    first: 10,
                    last: 12,
                },
            ],
            6,
            2,
        );
        assert_eq!(
            chunks,
            [
                ArticleRange { first: 7, last: 8 },
                ArticleRange { first: 9, last: 9 },
                ArticleRange {
                    first: 13,
                    last: 14
                },
                ArticleRange {
                    first: 15,
                    last: 15
                },
            ]
        );
    }

    #[test]
    fn limits_to_newest_unread_articles_and_handles_empty_groups() {
        assert_eq!(
            plan_unread_chunks(
                100,
                200,
                &[ArticleRange {
                    first: 100,
                    last: 190
                }],
                5,
                3
            ),
            [
                ArticleRange {
                    first: 196,
                    last: 198
                },
                ArticleRange {
                    first: 199,
                    last: 200
                },
            ]
        );
        assert!(plan_unread_chunks(0, 0, &[], 10, 5).is_empty());
        assert!(plan_unread_chunks(1, 10, &[], 0, 5).is_empty());
    }

    #[test]
    fn read_context_takes_the_newest_read_articles_inside_the_active_range() {
        let range = |first, last| ArticleRange { first, last };
        // Read: 1-20 (partly expired below 5), 30-31, 40; unread elsewhere.
        let read = [range(1, 20), range(30, 31), range(40, 40), range(90, 99)];
        assert_eq!(
            plan_read_context_chunks(5, 50, &read, 6, 2),
            [range(18, 19), range(20, 20), range(30, 31), range(40, 40)]
        );
        assert_eq!(
            plan_read_context_chunks(5, 50, &read, 100, 100),
            [range(5, 20), range(30, 31), range(40, 40)]
        );
        assert!(plan_read_context_chunks(5, 50, &read, 0, 2).is_empty());
        assert!(plan_read_context_chunks(5, 50, &[], 10, 2).is_empty());
        assert!(plan_read_context_chunks(0, 0, &read, 10, 2).is_empty());
    }

    #[test]
    fn combined_plan_joins_interleaved_read_and_unread_articles() {
        let read = (1..=19)
            .step_by(2)
            .map(|number| ArticleRange {
                first: number,
                last: number,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            plan_overview_chunks(1, 20, &read, 100, 100, 1_000),
            [ArticleRange { first: 1, last: 20 }]
        );
        assert_eq!(
            plan_overview_chunks(1, 20, &read, 100, 100, 8),
            [
                ArticleRange { first: 1, last: 8 },
                ArticleRange { first: 9, last: 16 },
                ArticleRange {
                    first: 17,
                    last: 20
                },
            ]
        );
    }

    #[test]
    fn combined_plan_keeps_gaps_left_by_the_limits() {
        let read = [ArticleRange { first: 1, last: 10 }];
        // Context covers 8-10 and unread covers 11-20; 1-7 is left out.
        assert_eq!(
            plan_overview_chunks(1, 20, &read, 100, 3, 1_000),
            [ArticleRange { first: 8, last: 20 }]
        );
        // With no context, read articles stay out of the plan.
        assert_eq!(
            plan_overview_chunks(1, 20, &read, 100, 0, 1_000),
            [ArticleRange {
                first: 11,
                last: 20
            }]
        );
        // Unread limited to 18-20 and context to 9-10: a real gap remains.
        assert_eq!(
            plan_overview_chunks(1, 20, &read, 3, 2, 1_000),
            [
                ArticleRange { first: 9, last: 10 },
                ArticleRange {
                    first: 18,
                    last: 20
                },
            ]
        );
        assert!(plan_overview_chunks(1, 20, &read, 0, 0, 1_000).is_empty());
        assert!(plan_overview_chunks(1, 20, &read, 10, 10, 0).is_empty());
    }

    #[test]
    fn avoids_overflow_at_maximum_article_number() {
        assert_eq!(
            plan_unread_chunks(u64::MAX - 2, u64::MAX, &[], 3, 2),
            [
                ArticleRange {
                    first: u64::MAX - 2,
                    last: u64::MAX - 1
                },
                ArticleRange {
                    first: u64::MAX,
                    last: u64::MAX
                },
            ]
        );
    }
}
