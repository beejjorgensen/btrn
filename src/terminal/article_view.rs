use crate::decoder::DecodedArticle;
use crate::state::thread_hierarchy::{
    HierarchyGrid, HierarchyRow, HIERARCHY_HEIGHT, HIERARCHY_WIDTH,
};
use crate::state::{SessionState, StateError};
use crate::terminal::selector;
use crossterm::{
    cursor::MoveTo,
    queue,
    style::{Attribute, SetAttribute},
    terminal::{Clear, ClearType},
};
use std::io::{self, Write};

const DISPLAYED_HEADERS: [&str; 3] = ["From", "Subject", "Date"];
const TAB_STOP: usize = 8;
/// Prefix of a header row continued from the row above.
const HEADER_CONTINUATION: &str = "+   ";
/// Columns between the header text and the thread window.
const HIERARCHY_GAP: usize = 1;
/// Narrowest header text area that still shows the thread window; narrower
/// terminals omit the window and keep full-width headers.
const MIN_HEADER_WIDTH_WITH_HIERARCHY: usize = 20;

/// Context for the article view's top line and prompt that does not come from
/// the article itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArticleViewContext<'a> {
    pub group: &'a str,
    pub article_number: u64,
    /// Unread articles in the group other than the one being shown.
    pub more_unread: usize,
    /// The group's highest server article number, from `GROUP`.
    pub group_last_article: u64,
}

/// Width-dependent layout of the whole article display: the top line,
/// filtered headers, a blank separator, and wrapped body rows, in one
/// continuous list so paging scrolls headers away with the body. This is
/// derived display data, never stored in the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleLayout {
    rows: Vec<String>,
    /// The thread window's rows, overlaid on the right of the first display
    /// rows. Empty when no window is shown.
    hierarchy: Vec<HierarchyRow>,
}

impl ArticleLayout {
    pub fn new(context: &ArticleViewContext<'_>, article: &DecodedArticle, width: u16) -> Self {
        let mut rows = vec![selector::fit_terminal_line(&top_line(context), width)];
        rows.extend(
            article_header_lines(article)
                .iter()
                .map(|line| selector::fit_terminal_line(line, width)),
        );
        rows.push(String::new());
        rows.extend(body_lines(&article.body, width));
        Self {
            rows,
            hierarchy: Vec::new(),
        }
    }

    /// Lays out the article with the thread window in the upper right. The
    /// top line and headers beside the window wrap rather than run into it,
    /// and the Subject header carries its `[N]` subject number. Terminals too
    /// narrow for both fall back to the layout without a window.
    pub fn with_hierarchy(
        context: &ArticleViewContext<'_>,
        article: &DecodedArticle,
        hierarchy: &HierarchyGrid,
        width: u16,
    ) -> Self {
        let full = usize::from(width);
        let Some(narrow) = full
            .checked_sub(HIERARCHY_WIDTH + HIERARCHY_GAP)
            .filter(|narrow| *narrow >= MIN_HEADER_WIDTH_WITH_HIERARCHY)
        else {
            return Self::new(context, article, width);
        };
        let row_width = |row: usize| if row < HIERARCHY_HEIGHT { narrow } else { full };

        let mut lines = vec![top_line(context)];
        lines.extend(header_lines(article, hierarchy.current_subject_index()));
        let mut rows: Vec<String> = Vec::new();
        for line in &lines {
            if rows.len() < HIERARCHY_HEIGHT {
                rows.extend(wrap_header(line, rows.len(), row_width));
            } else {
                rows.push(selector::fit_terminal_line(line, width));
            }
        }
        rows.push(String::new());
        // Keep body text out of the window's rows when headers are few.
        while rows.len() < HIERARCHY_HEIGHT {
            rows.push(String::new());
        }
        rows.extend(body_lines(&article.body, width));
        Self {
            rows,
            hierarchy: hierarchy.window(HIERARCHY_WIDTH, HIERARCHY_HEIGHT),
        }
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[String] {
        &self.rows
    }

    /// Article rows per page: the screen minus the prompt line. At least one
    /// row is always shown.
    pub fn rows_per_page(height: u16) -> usize {
        usize::from(height).saturating_sub(1).max(1)
    }

    pub fn page_count(&self, height: u16) -> usize {
        self.rows.len().div_ceil(Self::rows_per_page(height)).max(1)
    }

    /// A height at which the whole article fits on a single page.
    pub fn full_height(&self) -> u16 {
        u16::try_from(self.rows.len() + 1).unwrap_or(u16::MAX)
    }

    /// The rows shown on `page` and how many rows have been shown through it.
    /// Each page scrolls by the rows it adds, so a short last page still shows
    /// a full screen ending at the last row, keeping the prompt at the bottom.
    fn page_rows(&self, page: usize, height: u16) -> (&[String], usize) {
        let (start, end) = self.page_range(page, height);
        (&self.rows[start..end], end)
    }

    fn page_range(&self, page: usize, height: u16) -> (usize, usize) {
        let per_page = Self::rows_per_page(height);
        let end = page
            .saturating_add(1)
            .saturating_mul(per_page)
            .min(self.rows.len());
        (end.saturating_sub(per_page), end)
    }
}

/// Wraps a header beginning on display row `start_row` so each row fits
/// `row_width(row)` characters. Continuation rows begin with `+   `. Control
/// characters are shown as U+FFFD.
pub fn wrap_header(
    line: &str,
    start_row: usize,
    row_width: impl Fn(usize) -> usize,
) -> Vec<String> {
    let mut remaining = line
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'
            } else {
                character
            }
        })
        .peekable();
    let mut rows = Vec::new();
    let mut row = start_row;
    loop {
        let continuation = !rows.is_empty();
        let mut text = if continuation {
            HEADER_CONTINUATION.to_owned()
        } else {
            String::new()
        };
        let capacity = if continuation {
            row_width(row)
                .saturating_sub(HEADER_CONTINUATION.len())
                .max(1)
        } else {
            row_width(row).max(1)
        };
        text.extend(remaining.by_ref().take(capacity));
        rows.push(text);
        if remaining.peek().is_none() {
            return rows;
        }
        row += 1;
    }
}

/// Selects the From, Subject, Date, and Lines headers in display order. Lines
/// falls back to the body line count when the header is absent.
pub fn article_header_lines(article: &DecodedArticle) -> Vec<String> {
    header_lines(article, None)
}

/// Like [`article_header_lines`], but with a subject number the Subject header
/// is shown as `[N] subject`.
fn header_lines(article: &DecodedArticle, subject_index: Option<usize>) -> Vec<String> {
    let header = |name: &str| {
        article
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .map(|header| header.value.replace('\t', " "))
    };
    let mut lines = DISPLAYED_HEADERS
        .iter()
        .filter_map(|name| {
            header(name).map(|value| match subject_index {
                Some(index) if *name == "Subject" => format!("[{index}] {value}"),
                _ => format!("{name}: {value}"),
            })
        })
        .collect::<Vec<_>>();
    let line_count = header("Lines").unwrap_or_else(|| {
        if article.body.is_empty() {
            "0".to_owned()
        } else {
            article.body.split('\n').count().to_string()
        }
    });
    lines.push(format!("Lines: {line_count}"));
    lines
}

/// Splits a decoded body into terminal rows: tabs expand to 8-column stops,
/// control characters become U+FFFD, and long lines wrap at `width`.
pub fn body_lines(body: &str, width: u16) -> Vec<String> {
    if body.is_empty() {
        return Vec::new();
    }
    let width = usize::from(width).max(1);
    let mut rows = Vec::new();
    for line in body.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let mut row = String::new();
        let mut columns = 0;
        let mut push = |row: &mut String, columns: &mut usize, character: char| {
            if *columns == width {
                rows.push(std::mem::take(row));
                *columns = 0;
            }
            row.push(character);
            *columns += 1;
        };
        for character in line.chars() {
            if character == '\t' {
                let spaces = TAB_STOP - columns % TAB_STOP;
                for _ in 0..spaces {
                    push(&mut row, &mut columns, ' ');
                }
            } else if character.is_control() {
                push(&mut row, &mut columns, '\u{fffd}');
            } else {
                push(&mut row, &mut columns, character);
            }
        }
        rows.push(row);
    }
    rows
}

pub fn top_line(context: &ArticleViewContext<'_>) -> String {
    let mut line = format!("{} #{}", context.group, context.article_number);
    if context.more_unread > 0 {
        line.push_str(&format!(" ({} more)", context.more_unread));
    }
    line
}

pub fn prompt_line(
    context: &ArticleViewContext<'_>,
    layout: &ArticleLayout,
    page: usize,
    height: u16,
) -> String {
    let (_, shown_through) = layout.page_rows(page, height);
    let total = layout.rows.len();
    if shown_through < total {
        let percent = shown_through.saturating_mul(100) / total;
        format!("--MORE--({percent}%)")
    } else {
        format!(
            "End of art {} (of {}) [npq]",
            context.article_number, context.group_last_article
        )
    }
}

/// Renders one page of the article view. Pages past the end are clamped to the
/// last page.
pub fn render_article_page(
    output: &mut impl Write,
    context: &ArticleViewContext<'_>,
    layout: &ArticleLayout,
    page: usize,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let page = page.min(layout.page_count(height) - 1);
    queue!(output, Clear(ClearType::All), MoveTo(0, 0))?;
    let (start, end) = layout.page_range(page, height);
    for (index, row) in layout.rows[start..end].iter().enumerate() {
        output.write_all(row.as_bytes())?;
        if let Some(hierarchy) = layout.hierarchy.get(start + index) {
            let column = usize::from(width).saturating_sub(HIERARCHY_WIDTH);
            let padding = column.saturating_sub(row.chars().count());
            output.write_all(" ".repeat(padding).as_bytes())?;
            write_hierarchy_row(output, hierarchy)?;
        }
        output.write_all(b"\r\n")?;
    }
    selector::write_line(
        output,
        &prompt_line(context, layout, page, height),
        false,
        width,
        true,
    )?;
    output.flush()
}

fn write_hierarchy_row(output: &mut impl Write, row: &HierarchyRow) -> io::Result<()> {
    let mut inverse = false;
    let mut buffer = [0; 4];
    for cell in row {
        if cell.inverse != inverse {
            let attribute = if cell.inverse {
                Attribute::Reverse
            } else {
                Attribute::Reset
            };
            queue!(output, SetAttribute(attribute))?;
            inverse = cell.inverse;
        }
        output.write_all(cell.character.encode_utf8(&mut buffer).as_bytes())?;
    }
    if inverse {
        queue!(output, SetAttribute(Attribute::Reset))?;
    }
    Ok(())
}

/// Renders the first page of an article and only then marks it read.
pub fn present_article(
    output: &mut impl Write,
    state: &mut SessionState,
    context: &ArticleViewContext<'_>,
    layout: &ArticleLayout,
    width: u16,
    height: u16,
) -> Result<(), PresentArticleError> {
    render_article_page(output, context, layout, 0, width, height)
        .map_err(PresentArticleError::Render)?;
    state
        .mark_article_read(context.group, context.article_number)
        .map_err(PresentArticleError::State)
}

#[derive(Debug)]
pub enum PresentArticleError {
    Render(io::Error),
    State(StateError),
}

impl std::fmt::Display for PresentArticleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Render(error) => write!(formatter, "could not render article: {error}"),
            Self::State(error) => write!(formatter, "could not update article read state: {error}"),
        }
    }
}

impl std::error::Error for PresentArticleError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{decode_article, DecodedArticle, DecodedHeader};
    use crate::model::article::{HeaderField, RawArticle};
    use crate::state::thread_hierarchy::build_hierarchy;

    #[derive(Default)]
    struct TestWriter {
        bytes: Vec<u8>,
        fail_flush: bool,
    }

    impl Write for TestWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::other("flush failed"))
            } else {
                Ok(())
            }
        }
    }

    fn sample_article() -> DecodedArticle {
        let raw = RawArticle {
            headers: vec![HeaderField {
                name: b"Subject".to_vec(),
                value: b"hello\tthere\x1b[31m".to_vec(),
            }],
            body: vec![b"body".to_vec()],
        };
        decode_article(&raw)
    }

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

    fn context(group: &str) -> ArticleViewContext<'_> {
        ArticleViewContext {
            group,
            article_number: 9,
            more_unread: 3,
            group_last_article: 42,
        }
    }

    fn numbered_body(lines: usize) -> String {
        (1..=lines)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn marks_read_only_after_the_article_view_flushes_successfully() {
        let mut state = SessionState::default();
        let mut output = TestWriter::default();
        let article = sample_article();
        let layout = ArticleLayout::new(&context("comp.test"), &article, 80);
        present_article(
            &mut output,
            &mut state,
            &context("comp.test"),
            &layout,
            80,
            24,
        )
        .unwrap();

        assert!(state.is_read("comp.test", 9));
        let text = String::from_utf8(output.bytes).unwrap();
        assert!(!text
            .as_bytes()
            .windows(2)
            .any(|bytes| bytes[1] == b'\n' && bytes[0] != b'\r'));
        assert!(text.contains("comp.test #9 (3 more)"));
        assert!(text.contains("Subject: hello there\u{fffd}[31m"));
        assert!(text.contains("body"));
        assert!(text.contains("End of art 9 (of 42) [npq]"));
    }

    #[test]
    fn does_not_mark_read_when_render_or_flush_fails() {
        let mut state = SessionState::default();
        let mut output = TestWriter {
            fail_flush: true,
            ..TestWriter::default()
        };
        let article = sample_article();
        let layout = ArticleLayout::new(&context("comp.test"), &article, 80);
        assert!(matches!(
            present_article(
                &mut output,
                &mut state,
                &context("comp.test"),
                &layout,
                80,
                24
            ),
            Err(PresentArticleError::Render(_))
        ));
        assert!(!state.is_read("comp.test", 9));
        assert!(state.newsrc.entries.is_empty());
    }

    #[test]
    fn shows_only_selected_headers_in_display_order() {
        let article = article(
            &[
                ("Path", "news!not-shown"),
                ("date", "Thu, 19 Nov 2026 14:11:11 -0800"),
                ("Subject", "Re: topic"),
                ("Message-ID", "<id@example>"),
                ("From", "Someone <someone@example.com>"),
                ("Lines", "15"),
            ],
            "body",
        );
        assert_eq!(
            article_header_lines(&article),
            [
                "From: Someone <someone@example.com>",
                "Subject: Re: topic",
                "Date: Thu, 19 Nov 2026 14:11:11 -0800",
                "Lines: 15",
            ]
        );
    }

    #[test]
    fn counts_body_lines_when_lines_header_is_absent() {
        assert_eq!(
            article_header_lines(&article(&[("Subject", "s")], "a\nb\nc")),
            ["Subject: s", "Lines: 3"]
        );
        assert_eq!(article_header_lines(&article(&[], "")), ["Lines: 0"]);
    }

    #[test]
    fn body_rows_expand_tabs_sanitize_controls_and_wrap() {
        assert_eq!(body_lines("a\tb\r\nc\x07", 80), ["a       b", "c\u{fffd}"]);
        assert_eq!(body_lines("abcdefg", 3), ["abc", "def", "g"]);
        assert_eq!(body_lines("abcdef", 3), ["abc", "def"]);
        assert_eq!(
            body_lines("\u{e9}\u{e9}\u{e9}\u{e9}", 2),
            ["\u{e9}\u{e9}", "\u{e9}\u{e9}"]
        );
        assert_eq!(body_lines("a\n\nb", 80), ["a", "", "b"]);
        assert!(body_lines("", 80).is_empty());
    }

    fn numbered_layout() -> ArticleLayout {
        ArticleLayout::new(
            &context("comp.test"),
            &article(&[("Subject", "s")], &numbered_body(10)),
            80,
        )
    }

    #[test]
    fn layout_is_one_continuous_list_of_top_line_headers_blank_and_body() {
        let layout = numbered_layout();
        assert_eq!(
            &layout.rows()[..5],
            [
                "comp.test #9 (3 more)",
                "Subject: s",
                "Lines: 10",
                "",
                "line 1"
            ]
        );
        assert_eq!(layout.rows().len(), 14);
    }

    #[test]
    fn pages_divide_the_whole_display_by_rows_above_the_prompt() {
        let layout = numbered_layout();
        assert_eq!(ArticleLayout::rows_per_page(10), 9);
        assert_eq!(layout.page_count(10), 2);
        assert_eq!(layout.page_count(15), 1);
        assert_eq!(layout.page_count(14), 2);
        assert_eq!(layout.page_count(layout.full_height()), 1);
        // A screen with no room above the prompt still shows one row per page.
        assert_eq!(ArticleLayout::rows_per_page(1), 1);
        assert_eq!(layout.page_count(1), 14);
        let empty = ArticleLayout::new(&context("comp.test"), &article(&[], ""), 80);
        assert_eq!(empty.page_count(10), 1);
    }

    #[test]
    fn long_header_lines_are_truncated_to_one_row() {
        let layout = ArticleLayout::new(
            &context("comp.test"),
            &article(&[("Subject", "a very long subject line")], "body"),
            12,
        );
        let subject = &layout.rows()[1];
        assert!(subject.starts_with("Subject: "));
        assert!(subject.ends_with('…'));
        assert!(subject.chars().count() <= 12);
        assert_eq!(layout.rows().len(), 5);
    }

    #[test]
    fn prompt_shows_more_percentage_of_the_whole_display_until_the_last_page() {
        let layout = numbered_layout();
        let context = context("comp.test");
        assert_eq!(prompt_line(&context, &layout, 0, 6), "--MORE--(35%)");
        assert_eq!(prompt_line(&context, &layout, 1, 6), "--MORE--(71%)");
        assert_eq!(
            prompt_line(&context, &layout, 2, 6),
            "End of art 9 (of 42) [npq]"
        );
    }

    #[test]
    fn later_pages_scroll_the_headers_away_and_clamp_past_the_end() {
        let layout = numbered_layout();
        let mut output = Vec::new();
        render_article_page(&mut output, &context("comp.test"), &layout, 0, 80, 10).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("comp.test #9 (3 more)\r\nSubject: s\r\n"));
        assert!(text.contains("line 5\r\n"));
        assert!(!text.contains("line 6"));
        assert!(text.contains("--MORE--(64%)"));

        let mut output = Vec::new();
        render_article_page(&mut output, &context("comp.test"), &layout, 1, 80, 10).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(!text.contains("comp.test #9"));
        assert!(!text.contains("Subject:"));
        assert!(!text.contains("Lines: 10"));
        assert!(!text.contains("line 1\r\n"));
        assert!(text.contains("line 2\r\nline 3\r\n"));
        assert!(text.contains("line 9\r\nline 10\r\n"));
        assert!(text.contains("End of art 9 (of 42) [npq]"));

        let mut output = Vec::new();
        render_article_page(&mut output, &context("comp.test"), &layout, 99, 80, 10).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), text);
    }

    #[test]
    fn short_last_page_still_fills_the_screen_with_the_prompt_on_the_bottom_row() {
        let layout = numbered_layout();
        let mut output = Vec::new();
        render_article_page(&mut output, &context("comp.test"), &layout, 1, 80, 10).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(text.matches("\r\n").count(), 9);
        assert!(text.ends_with("End of art 9 (of 42) [npq]\u{1b}[0m"));
    }

    #[test]
    fn single_page_article_puts_the_prompt_right_after_its_content() {
        let layout = numbered_layout();
        let mut output = Vec::new();
        render_article_page(&mut output, &context("comp.test"), &layout, 0, 80, 24).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert_eq!(text.matches("\r\n").count(), 14);
        assert!(text.contains("comp.test #9 (3 more)\r\n"));
        assert!(text.contains("line 10\r\n"));
        assert!(text.contains("End of art 9 (of 42) [npq]"));
    }

    #[test]
    fn top_line_omits_more_count_when_nothing_else_is_unread() {
        let context = ArticleViewContext {
            more_unread: 0,
            ..context("comp.test")
        };
        assert_eq!(top_line(&context), "comp.test #9");
    }

    fn hierarchy_session() -> SessionState {
        use crate::model::article::{ArticleMetadata, MessageId};
        use crate::state::LoadedArticle;
        let article = |number: u64, id: &str, parent: Option<&str>| LoadedArticle {
            article_number: number,
            metadata: ArticleMetadata {
                message_id: MessageId::parse(format!("<{id}>").as_bytes()),
                references: parent
                    .map(|parent| vec![MessageId::parse(format!("<{parent}>").as_bytes()).unwrap()])
                    .unwrap_or_default(),
                in_reply_to: None,
                from: None,
                subject: Some(b"Re: topic".to_vec()),
                date: None,
                issues: Vec::new(),
            },
        };
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [
                    article(1, "a", None),
                    article(9, "b", Some("a")),
                    article(10, "c", Some("a")),
                ],
            )
            .unwrap();
        state
    }

    fn hierarchy_layout(article: &DecodedArticle, width: u16) -> ArticleLayout {
        let state = hierarchy_session();
        let grid = build_hierarchy(&state, "comp.test", 1, 9, Some(1)).unwrap();
        ArticleLayout::with_hierarchy(&context("comp.test"), article, &grid, width)
    }

    fn strip_escapes(text: &str) -> String {
        let mut plain = String::new();
        let mut characters = text.chars();
        while let Some(character) = characters.next() {
            if character == '\u{1b}' {
                for next in characters.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                plain.push(character);
            }
        }
        plain
    }

    #[test]
    fn header_that_fits_is_not_wrapped() {
        assert_eq!(wrap_header("Lines: 3", 0, |_| 20), ["Lines: 3"]);
    }

    #[test]
    fn long_header_wraps_with_continuation_prefix() {
        assert_eq!(
            wrap_header("Subject: abcdefghij", 0, |_| 12),
            ["Subject: abc", "+   defghij"]
        );
        assert_eq!(
            wrap_header("0123456789abcdefghij", 2, |_| 8),
            ["01234567", "+   89ab", "+   cdef", "+   ghij"]
        );
        assert_eq!(wrap_header("a\u{7}b", 0, |_| 10), ["a\u{fffd}b"]);
    }

    #[test]
    fn wrapped_header_widens_once_past_the_window_rows() {
        let row_width = |row: usize| if row < HIERARCHY_HEIGHT { 6 } else { 40 };
        assert_eq!(
            wrap_header("abcdefghijklmnopqrstuvwxyz", 4, row_width),
            ["abcdef", "+   gh", "+   ijklmnopqrstuvwxyz"]
        );
    }

    #[test]
    fn subject_header_shows_its_subject_number() {
        let layout = hierarchy_layout(&article(&[("Subject", "Re: topic")], "body"), 80);
        assert_eq!(layout.rows()[1], "[1] Re: topic");
    }

    #[test]
    fn long_headers_beside_the_window_wrap_and_push_the_rest_down() {
        // 46 columns leave 20 for headers beside the window.
        let layout = hierarchy_layout(
            &article(
                &[
                    ("From", "Someone <someone@example.com>"),
                    ("Subject", "topic"),
                    ("Date", "today"),
                ],
                "line 1\nline 2",
            ),
            46,
        );
        assert_eq!(
            layout.rows(),
            [
                "comp.test #9 (3 more",
                "+   )",
                "From: Someone <someo",
                "+   ne@example.com>",
                "[1] topic",
                "Date: today",
                "Lines: 2",
                "",
                "line 1",
                "line 2",
            ]
        );
        assert_eq!(layout.page_count(11), 1);
        assert_eq!(layout.page_count(10), 2);
    }

    #[test]
    fn few_headers_pad_the_window_rows_before_the_body() {
        let layout = hierarchy_layout(&article(&[], "body"), 80);
        assert_eq!(
            layout.rows(),
            ["comp.test #9 (3 more)", "Lines: 1", "", "", "", "", "body"]
        );
    }

    #[test]
    fn narrow_terminal_omits_the_window() {
        let layout = hierarchy_layout(&article(&[("Subject", "topic")], "body"), 45);
        assert_eq!(layout.rows()[1], "Subject: topic");
        assert!(layout.hierarchy.is_empty());
    }

    #[test]
    fn renders_the_thread_window_in_the_upper_right_of_the_first_page() {
        let layout = hierarchy_layout(
            &article(
                &[
                    ("From", "Someone <someone@example.com>"),
                    ("Subject", "Re: topic"),
                    ("Date", "Thu, 19 Nov 2026 14:11:11 -0800"),
                ],
                &numbered_body(3),
            ),
            60,
        );
        let mut output = Vec::new();
        render_article_page(&mut output, &context("comp.test"), &layout, 0, 60, 24).unwrap();
        let raw = String::from_utf8(output).unwrap();
        let plain = strip_escapes(&raw);
        let rows = plain.split("\r\n").collect::<Vec<_>>();
        assert_eq!(
            rows,
            [
                "comp.test #9 (3 more)              (1)+-[1]",
                "From: Someone <someone@example.com    \\-(1)",
                "+   >",
                "[1] Re: topic",
                "Date: Thu, 19 Nov 2026 14:11:11 -0",
                "+   800",
                "Lines: 3",
                "",
                "line 1",
                "line 2",
                "line 3",
                "End of art 9 (of 42) [npq]",
            ]
        );
        // The current article (9) is fully inverse; the previous (1) has only
        // its number inverse.
        assert!(raw.contains("(\u{1b}[7m1\u{1b}[0m)+-\u{1b}[7m[1]\u{1b}[0m"));
    }

    #[test]
    fn the_window_scrolls_away_with_the_headers() {
        let layout = hierarchy_layout(&article(&[("Subject", "s")], &numbered_body(20)), 80);
        let mut output = Vec::new();
        render_article_page(&mut output, &context("comp.test"), &layout, 1, 80, 10).unwrap();
        let plain = strip_escapes(&String::from_utf8(output).unwrap());
        assert!(!plain.contains("+-"));
        assert!(!plain.contains("\\-"));
    }
}
