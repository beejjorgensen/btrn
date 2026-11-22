//! The thread selector: pages of threads with unread articles, each with a
//! selection letter, its unread authors, and its lead subject.

use crate::decoder::decode_header_value;
use crate::state::thread_list::{ThreadList, ThreadRow};
use crate::state::SessionState;
use crate::terminal::selector;
use crossterm::queue;
use crossterm::terminal::{Clear, ClearType};
use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, Write};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ThreadEntry {
    pub(crate) index: usize,
    pub(crate) authors: Vec<String>,
    omitted_authors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ThreadPage {
    pub(crate) entries: Vec<ThreadEntry>,
    end_index: usize,
}

pub(crate) fn thread_pages(
    threads: &ThreadList,
    state: &SessionState,
    height: u16,
) -> Vec<ThreadPage> {
    const MAX_PAGE_THREADS: usize = 35;
    let mut pages = Vec::new();
    let capacity = usize::from(height).saturating_sub(4);
    if capacity == 0 {
        return vec![ThreadPage::default()];
    }
    let mut page = ThreadPage::default();
    let mut used = 0;
    for (index, row) in threads.rows.iter().enumerate() {
        if row.unread_count == 0 {
            continue;
        }
        if page.entries.len() == MAX_PAGE_THREADS {
            page.end_index = index;
            pages.push(page);
            page = ThreadPage::default();
            used = 0;
        }
        let authors = thread_authors(row, state);
        let entry_lines = authors.len().max(1);
        if used > 0 && used + entry_lines > capacity {
            page.end_index = index;
            pages.push(page);
            page = ThreadPage::default();
            used = 0;
        }

        if entry_lines > capacity {
            let visible_authors = capacity
                .saturating_sub(2)
                .min(authors.len().saturating_sub(1));
            let omitted_authors = if capacity >= 2 {
                authors.len().saturating_sub(1 + visible_authors)
            } else {
                0
            };
            page.entries.push(ThreadEntry {
                index,
                authors: authors.into_iter().take(1 + visible_authors).collect(),
                omitted_authors,
            });
            page.end_index = index + 1;
            pages.push(page);
            page = ThreadPage::default();
            used = 0;
        } else {
            page.entries.push(ThreadEntry {
                index,
                authors,
                omitted_authors: 0,
            });
            used += entry_lines;
            page.end_index = index + 1;
        }
    }
    if !page.entries.is_empty() || pages.is_empty() {
        pages.push(page);
    }
    pages
}

fn thread_authors(row: &ThreadRow, state: &SessionState) -> Vec<String> {
    let mut article_numbers = row.article_numbers.clone();
    if let Some(first_unread) = article_numbers
        .iter()
        .position(|number| !state.is_read(&row.group_name, *number))
    {
        article_numbers.rotate_left(first_unread);
    }
    article_numbers
        .iter()
        .filter(|number| !state.is_read(&row.group_name, **number))
        .map(|number| {
            state
                .groups
                .get(&row.group_name)
                .and_then(|group| group.articles.get(number))
                .and_then(|metadata| metadata.from.as_deref())
                .map(thread_author_name)
                .unwrap_or_else(|| "(unknown author)".to_owned())
        })
        .collect()
}

fn thread_author_name(value: &[u8]) -> String {
    let decoded = decode_header_value(b"From", value).0;
    let decoded = decoded.trim();
    let name = if let Some(opening) = decoded.rfind('<') {
        let display_name = decoded[..opening].trim();
        if display_name.is_empty() {
            email_username(decoded[opening + 1..].trim_end_matches('>').trim())
        } else {
            display_name
        }
    } else if let Some(opening) = decoded.rfind('(') {
        let trailing = decoded[opening + 1..].trim_end_matches(')').trim();
        if decoded[..opening].contains('@') && !trailing.is_empty() {
            trailing
        } else if decoded.contains('@') {
            email_username(decoded)
        } else {
            decoded
        }
    } else if decoded.contains('@') {
        email_username(decoded)
    } else {
        decoded
    };
    name.trim().trim_matches('"').trim().to_owned()
}

fn email_username(address: &str) -> &str {
    address
        .split_once('@')
        .map_or(address, |(username, _)| username)
        .trim()
}

fn truncate_thread_author(author: &str) -> String {
    const AUTHOR_COLUMN_WIDTH: usize = 18;
    let mut characters = author.chars();
    let mut truncated = characters
        .by_ref()
        .take(AUTHOR_COLUMN_WIDTH)
        .collect::<String>();
    if characters.next().is_some() {
        truncated.truncate(
            truncated
                .char_indices()
                .nth(AUTHOR_COLUMN_WIDTH - 3)
                .map_or(truncated.len(), |(index, _)| index),
        );
        truncated.push_str("...");
    }
    truncated
}

fn thread_heading(
    letter: char,
    marked: bool,
    author: &str,
    unread_count: usize,
    lead: &str,
    unresolved: &str,
) -> String {
    format!(
        "{}{} {:<18} {:>3}  >{}{}",
        letter,
        if marked && letter != ' ' { "+" } else { " " },
        truncate_thread_author(author),
        unread_count.min(999),
        lead,
        unresolved
    )
}

pub(crate) fn assigned_thread_letter(
    page: &ThreadPage,
    threads: &ThreadList,
    row_index: usize,
) -> Option<char> {
    let mut letter_index = 0;
    for entry in &page.entries {
        if threads.rows.get(entry.index)?.unread_count > 0 {
            if entry.index == row_index {
                return selector::selection_letter(letter_index);
            }
            letter_index += 1;
        }
    }
    None
}

pub(crate) fn page_thread_row_y(page: &ThreadPage, row_index: usize) -> Option<u16> {
    let mut y = 2usize;
    for entry in &page.entries {
        if entry.index == row_index {
            return u16::try_from(y).ok();
        }
        y = y
            .checked_add(1 + entry.authors.len().saturating_sub(1))?
            .checked_add(usize::from(entry.omitted_authors > 0))?;
    }
    None
}

/// Draws the whole thread-selector screen for page `page_index`, showing a
/// `+` beside each thread whose root is in `marked_threads`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_thread_list(
    output: &mut impl Write,
    threads: &ThreadList,
    marked_threads: &BTreeSet<u64>,
    page_index: usize,
    state: &SessionState,
    pages: &[ThreadPage],
    width: u16,
    height: u16,
) -> io::Result<()> {
    if height == 0 {
        return output.flush();
    }
    let page_index = page_index.min(pages.len().saturating_sub(1));
    let page = pages.get(page_index).cloned().unwrap_or_default();
    let page_position =
        selector::page_label(page_index, pages.len(), page.end_index, threads.rows.len());
    let article_count = threads
        .rows
        .iter()
        .map(|row| row.unread_count)
        .sum::<usize>();
    queue!(
        output,
        Clear(ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    )?;
    write_terminal_line(
        output,
        format_args!("{}", threads.group_name),
        false,
        width,
        true,
    )?;
    write_terminal_line(
        output,
        format_args!("          {article_count} articles"),
        height >= 2,
        width,
        false,
    )?;
    if height < 4 {
        if height >= 2 {
            queue!(
                output,
                crossterm::cursor::MoveTo(0, height.saturating_sub(1))
            )?;
            write_terminal_line(
                output,
                format_args!("-- {page_position} --"),
                false,
                width,
                true,
            )?;
        }
        return output.flush();
    }
    output.write_all(b"\r\n")?;
    let mut list_lines = 0usize;
    if threads.rows.is_empty() {
        write_terminal_line(output, format_args!("No threads"), true, width, false)?;
        list_lines = 1;
    }
    for entry in &page.entries {
        let row = &threads.rows[entry.index];
        let letter = assigned_thread_letter(&page, threads, entry.index).unwrap_or(' ');
        let marked = marked_threads.contains(&row.root_article_number);
        let first_unread = row
            .article_numbers
            .iter()
            .find(|number| !state.is_read(&row.group_name, **number))
            .copied()
            .or_else(|| row.article_numbers.first().copied());
        let author = entry
            .authors
            .first()
            .map(String::as_str)
            .unwrap_or("(unknown author)");
        let unresolved = if row.has_unresolved_parent {
            " (?)"
        } else {
            ""
        };
        let lead = first_unread
            .and_then(|number| {
                state
                    .groups
                    .get(&row.group_name)
                    .and_then(|group| group.articles.get(&number))
            })
            .and_then(|metadata| metadata.subject.as_deref())
            .map(|subject| decode_header_value(b"Subject", subject).0)
            .unwrap_or_else(|| row.subject.clone());
        let line = thread_heading(letter, marked, author, row.unread_count, &lead, unresolved);
        write_terminal_line(output, format_args!("{line}"), true, width, false)?;
        list_lines += 1;
        for author in entry.authors.iter().skip(1) {
            write_terminal_line(output, format_args!("   {author}"), true, width, false)?;
            list_lines += 1;
        }
        if entry.omitted_authors > 0 {
            write_terminal_line(
                output,
                format_args!("   ... etc. [{} lines total]", entry.omitted_authors),
                true,
                width,
                false,
            )?;
            list_lines += 1;
        }
    }
    let prompt_row = 3usize
        .saturating_add(list_lines)
        .min(usize::from(height.saturating_sub(1)));
    queue!(
        output,
        crossterm::cursor::MoveTo(0, u16::try_from(prompt_row).unwrap_or(u16::MAX))
    )?;
    write_terminal_line(
        output,
        format_args!(
            "-- Select Threads (date order) -- {page_position} --  < > / Space: page  q: quit"
        ),
        false,
        width,
        true,
    )?;
    output.flush()
}

fn write_terminal_line(
    output: &mut impl Write,
    arguments: fmt::Arguments<'_>,
    terminate: bool,
    width: u16,
    inverse: bool,
) -> io::Result<()> {
    let mut line = String::new();
    fmt::write(&mut line, arguments)
        .map_err(|_| io::Error::other("could not format terminal line"))?;
    selector::write_line(output, &line, terminate, width, inverse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::article::{ArticleMetadata, MessageId};
    use crate::state::LoadedArticle;

    fn loaded_article(number: u64, id: &str, parent: Option<&str>) -> LoadedArticle {
        LoadedArticle {
            article_number: number,
            metadata: ArticleMetadata {
                message_id: MessageId::parse(format!("<{id}>").as_bytes()),
                references: parent
                    .map(|parent| vec![MessageId::parse(format!("<{parent}>").as_bytes()).unwrap()])
                    .unwrap_or_default(),
                in_reply_to: None,
                from: Some(format!("author-{id}").into_bytes()),
                subject: Some(format!("subject-{id}").into_bytes()),
                date: None,
                issues: Vec::new(),
            },
        }
    }

    fn session_with_threads(count: u64) -> SessionState {
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                (1..=count).map(|number| loaded_article(number, &format!("thread-{number}"), None)),
            )
            .unwrap();
        state
    }

    /// The thread list as the reader shows it: only threads with unread
    /// articles.
    fn unread_threads(state: &SessionState) -> ThreadList {
        let mut threads = ThreadList::from_session(state, "comp.test").unwrap();
        threads.rows.retain(|row| row.unread_count > 0);
        threads
    }

    #[test]
    fn incremental_mark_update_positions_cursor_and_writes_only_the_marker() {
        let data = session_with_threads(5);
        let threads = unread_threads(&data);
        let pages = thread_pages(&threads, &data, 10);
        assert_eq!(page_thread_row_y(&pages[0], 2), Some(4));

        let mut output = Vec::new();
        selector::write_mark(&mut output, 1, 3, true).unwrap();
        assert_eq!(output, b"\x1b7\x1b[4;2H+\x1b8");

        output.clear();
        selector::write_mark(&mut output, 1, 3, false).unwrap();
        assert_eq!(output, b"\x1b7\x1b[4;2H \x1b8");
        assert!(!output.windows(4).any(|bytes| bytes == b"\x1b[2J"));
    }

    #[test]
    fn thread_author_names_omit_email_addresses() {
        assert_eq!(
            thread_author_name(b"=?UTF-8?Q?Jane_Doe?= <jane@example.com>"),
            "Jane Doe"
        );
        assert_eq!(
            thread_author_name(b"jane@example.com (Jane Doe)"),
            "Jane Doe"
        );
        assert_eq!(thread_author_name(b"jane@example.com"), "jane");
    }

    #[test]
    fn thread_heading_aligns_and_caps_the_article_count() {
        let line = thread_heading(
            'a',
            false,
            "A Very Long Author Display Name",
            1_234,
            "topic",
            "",
        );

        assert_eq!(&line[22..25], "999");
        assert!(line.starts_with("a  A Very Long Aut... 999"));
        assert!(!line.contains(" 1234 "));
    }

    #[test]
    fn thread_rendering_uses_author_display_name_without_email() {
        let mut data = session_with_threads(1);
        data.groups
            .get_mut("comp.test")
            .unwrap()
            .articles
            .get_mut(&1)
            .unwrap()
            .from = Some(b"Alice Example <alice@example.com>".to_vec());
        let threads = unread_threads(&data);
        let pages = thread_pages(&threads, &data, 10);
        let mut output = Vec::new();

        render_thread_list(
            &mut output,
            &threads,
            &BTreeSet::new(),
            0,
            &data,
            &pages,
            80,
            10,
        )
        .unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("articles\r\n\r\n"));
        assert!(rendered.contains("Alice Example"));
        assert!(!rendered.contains("alice@example.com"));
        assert!(rendered.contains("\u{1b}[5;1H"));
    }

    #[test]
    fn full_thread_page_keeps_one_blank_row_before_bottom_prompt() {
        let data = session_with_threads(4);
        let threads = unread_threads(&data);
        let pages = thread_pages(&threads, &data, 8);
        let mut output = Vec::new();

        render_thread_list(
            &mut output,
            &threads,
            &BTreeSet::new(),
            0,
            &data,
            &pages,
            80,
            8,
        )
        .unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert_eq!(pages[0].entries.len(), 4);
        assert!(rendered.contains("\u{1b}[8;1H"));
    }

    #[test]
    fn thread_row_position_accounts_for_multiline_entries() {
        let mut data = SessionState::default();
        data.load_articles(
            "comp.test",
            [
                loaded_article(10, "root", None),
                loaded_article(11, "reply", Some("root")),
                loaded_article(1, "other", None),
            ],
        )
        .unwrap();
        let threads = ThreadList::from_session(&data, "comp.test").unwrap();
        let pages = thread_pages(&threads, &data, 7);

        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].entries.len(), 2);
        assert_eq!(
            page_thread_row_y(&pages[0], pages[0].entries[0].index),
            Some(2)
        );
        assert_eq!(
            page_thread_row_y(&pages[0], pages[0].entries[1].index),
            Some(4)
        );
    }

    #[test]
    fn thread_selector_uses_digits_after_available_letters() {
        let data = session_with_threads(36);
        let threads = unread_threads(&data);
        let pages = thread_pages(&threads, &data, 50);

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].entries.len(), 35);
        assert_eq!(assigned_thread_letter(&pages[0], &threads, 34), Some('9'));
        assert_eq!(assigned_thread_letter(&pages[1], &threads, 35), Some('a'));
    }

    #[test]
    fn thread_pages_keep_complete_entries_and_truncate_a_too_long_thread() {
        let mut data = SessionState::default();
        data.load_articles(
            "comp.test",
            [
                loaded_article(10, "root", None),
                loaded_article(11, "reply-1", Some("root")),
                loaded_article(12, "reply-2", Some("reply-1")),
                loaded_article(13, "reply-3", Some("reply-2")),
                loaded_article(1, "other", None),
            ],
        )
        .unwrap();
        let threads = ThreadList::from_session(&data, "comp.test").unwrap();
        let pages = thread_pages(&threads, &data, 7);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].entries[0].index, 0);
        assert_eq!(pages[0].entries[0].omitted_authors, 2);
        assert_eq!(pages[1].entries[0].index, 1);

        let mut output = Vec::new();
        render_thread_list(
            &mut output,
            &threads,
            &BTreeSet::new(),
            0,
            &data,
            &pages,
            80,
            7,
        )
        .unwrap();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("... etc. [2 lines total]"));
        assert!(rendered.contains("\r\n   author-reply-1\r\n"));
        assert!(!rendered.contains("author-reply-3"));
        assert!(rendered.contains("Top 50%"));
        assert_eq!(rendered.matches("\r\n").count(), 5);
        assert!(!rendered.ends_with("\r\n"));
    }

    #[test]
    fn page_breaks_before_a_complete_thread_that_would_not_fit() {
        let mut data = SessionState::default();
        data.load_articles(
            "comp.test",
            [
                loaded_article(10, "root", None),
                loaded_article(11, "reply", Some("root")),
                loaded_article(1, "other", None),
            ],
        )
        .unwrap();
        let threads = ThreadList::from_session(&data, "comp.test").unwrap();
        let pages = thread_pages(&threads, &data, 6);

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].entries.len(), 1);
        assert_eq!(pages[0].entries[0].index, 0);
        assert_eq!(pages[0].entries[0].omitted_authors, 0);
        assert_eq!(pages[1].entries[0].index, 1);
    }

    #[test]
    fn fully_read_threads_have_no_letter_assignment() {
        let mut data = session_with_threads(1);
        data.mark_article_read("comp.test", 1).unwrap();
        let threads = unread_threads(&data);
        let pages = thread_pages(&threads, &data, 5);
        let mut output = Vec::new();

        render_thread_list(
            &mut output,
            &threads,
            &BTreeSet::new(),
            0,
            &data,
            &pages,
            80,
            5,
        )
        .unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert!(!rendered.contains("author-thread-1"));
        assert!(pages[0].entries.is_empty());
        assert_eq!(assigned_thread_letter(&pages[0], &threads, 0), None);
    }
}
