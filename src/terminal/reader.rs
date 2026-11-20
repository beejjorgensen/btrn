use crate::decoder;
use crate::nntp::{ArticleError, NntpSession};
use crate::state::article_navigation::ArticleNavigation;
use crate::state::thread_list::ThreadList;
use crate::state::{SessionState, StateError};
use crate::terminal::article_view::present_article;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderOperation {
    Quit,
    ToggleThread(char),
    NextPage,
    PreviousPage,
    StartSelectedThreads,
    CloseArticle,
    NextArticle,
    Redraw,
    Ignore,
}

pub fn operation_for_key(screen: ReaderScreen, key: KeyEvent) -> ReaderOperation {
    if key.kind != KeyEventKind::Press {
        return ReaderOperation::Ignore;
    }
    match (screen, key.code) {
        (_, KeyCode::Char('q')) | (_, KeyCode::Esc) => ReaderOperation::Quit,
        (ReaderScreen::Threads, KeyCode::Char('>')) => ReaderOperation::NextPage,
        (ReaderScreen::Threads, KeyCode::Char('<')) => ReaderOperation::PreviousPage,
        (ReaderScreen::Threads, KeyCode::Char(' ')) => ReaderOperation::StartSelectedThreads,
        (ReaderScreen::Article, KeyCode::Char(' ')) => ReaderOperation::NextArticle,
        (ReaderScreen::Article, KeyCode::Backspace) => ReaderOperation::CloseArticle,
        (ReaderScreen::Article, KeyCode::Char('r')) => ReaderOperation::Redraw,
        (ReaderScreen::Threads, KeyCode::Char(letter))
            if letter.is_ascii_alphabetic() && letter != 'q' =>
        {
            ReaderOperation::ToggleThread(letter.to_ascii_lowercase())
        }
        _ => ReaderOperation::Ignore,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderScreen {
    Threads,
    Article,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReaderState {
    pub threads: ThreadList,
    pub article_navigation: Option<ArticleNavigation>,
    thread_page: usize,
    marked_threads: BTreeSet<u64>,
    reading_threads: Vec<u64>,
    reading_thread_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReaderStateError {
    State(StateError),
    NoArticleSelected,
}

impl std::fmt::Display for ReaderStateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(error) => write!(formatter, "{error}"),
            Self::NoArticleSelected => write!(formatter, "thread has no loaded articles"),
        }
    }
}

impl std::error::Error for ReaderStateError {}

impl ReaderState {
    pub fn new(threads: ThreadList) -> Self {
        Self {
            threads,
            article_navigation: None,
            thread_page: 0,
            marked_threads: BTreeSet::new(),
            reading_threads: Vec::new(),
            reading_thread_index: 0,
        }
    }

    pub fn screen(&self) -> ReaderScreen {
        if self.article_navigation.is_some() {
            ReaderScreen::Article
        } else {
            ReaderScreen::Threads
        }
    }

    pub fn selected_article_number(&self) -> Option<u64> {
        self.article_navigation
            .as_ref()
            .and_then(ArticleNavigation::selected_article_number)
    }

    fn toggle_thread(&mut self, root_article_number: u64) {
        if !self.marked_threads.insert(root_article_number) {
            self.marked_threads.remove(&root_article_number);
        }
    }

    fn toggle_visible_thread(&mut self, letter: char, page: &ThreadPage) {
        let Some(index) = page.entries.iter().find_map(|entry| {
            let row = self.threads.rows.get(entry.index)?;
            (row.unread_count > 0
                && assigned_thread_letter(page, &self.threads, entry.index) == Some(letter))
            .then_some(entry.index)
        }) else {
            return;
        };
        if let Some(row) = self.threads.rows.get(index) {
            self.toggle_thread(row.root_article_number);
        }
    }

    fn next_page(&mut self, page_count: usize) {
        if self.thread_page + 1 < page_count {
            self.thread_page += 1;
        }
    }

    fn previous_page(&mut self) {
        self.thread_page = self.thread_page.saturating_sub(1);
    }

    fn normalize_page(&mut self, page_count: usize) {
        self.thread_page = self.thread_page.min(page_count.saturating_sub(1));
    }

    fn start_reading(&mut self, state: &SessionState) -> Result<(), ReaderStateError> {
        self.reading_threads = if self.marked_threads.is_empty() {
            self.threads
                .rows
                .iter()
                .map(|row| row.root_article_number)
                .collect()
        } else {
            self.threads
                .rows
                .iter()
                .filter(|row| self.marked_threads.contains(&row.root_article_number))
                .map(|row| row.root_article_number)
                .collect()
        };
        self.reading_thread_index = 0;
        if self.reading_threads.is_empty() {
            self.article_navigation = None;
            return Ok(());
        }
        self.open_reading_thread(state)
    }

    fn open_reading_thread(&mut self, state: &SessionState) -> Result<(), ReaderStateError> {
        let Some(root) = self.reading_threads.get(self.reading_thread_index).copied() else {
            self.article_navigation = None;
            return Err(ReaderStateError::NoArticleSelected);
        };
        self.article_navigation = Some(
            ArticleNavigation::from_thread(state, &self.threads.group_name, root)
                .map_err(ReaderStateError::State)?,
        );
        if self.selected_article_number().is_none() {
            return Err(ReaderStateError::NoArticleSelected);
        }
        Ok(())
    }

    pub fn next_article(&mut self) -> Option<u64> {
        self.article_navigation.as_mut()?.next_article()
    }

    fn advance_to_next_thread(&mut self, state: &SessionState) -> Result<bool, ReaderStateError> {
        if self.reading_thread_index + 1 >= self.reading_threads.len() {
            self.article_navigation = None;
            self.reading_threads.clear();
            return Ok(false);
        }
        self.reading_thread_index += 1;
        self.open_reading_thread(state)?;
        Ok(true)
    }

    pub fn close_article(&mut self, state: &SessionState) {
        self.article_navigation = None;
        self.reading_threads.clear();
        self.reading_thread_index = 0;
        if let Ok(updated) = ThreadList::from_session(state, &self.threads.group_name) {
            self.threads = updated;
            self.marked_threads.retain(|root| {
                self.threads
                    .rows
                    .iter()
                    .any(|row| row.root_article_number == *root)
            });
        }
    }
}

pub fn run_interactive(
    session: &mut NntpSession,
    state: &mut SessionState,
    mut reader_state: ReaderState,
) -> Result<(), ReaderError> {
    let mut terminal = TerminalSession::enter()?;
    let mut redraw = true;
    loop {
        if redraw {
            match reader_state.screen() {
                ReaderScreen::Threads => {
                    let (width, height) = terminal::size()?;
                    let pages = thread_pages(&reader_state.threads, state, height);
                    reader_state.normalize_page(pages.len());
                    render_thread_list(
                        &mut terminal.output,
                        &reader_state,
                        state,
                        &pages,
                        width,
                        height,
                    )?;
                }
                ReaderScreen::Article => {
                    let article_number = reader_state
                        .selected_article_number()
                        .ok_or(ReaderError::State(ReaderStateError::NoArticleSelected))?;
                    let group = reader_state.threads.group_name.clone();
                    let article = fetch_decoded_article(session, article_number)?;
                    present_article(
                        &mut terminal.output,
                        state,
                        &group,
                        article_number,
                        &article,
                    )
                    .map_err(ReaderError::Present)?;
                    write_terminal_line(
                        &mut terminal.output,
                        format_args!("q: threads  space: next article"),
                        false,
                        terminal::size()?.0,
                    )?;
                    terminal.output.flush()?;
                }
            }
            redraw = false;
        }

        let event = event::read()?;
        let key = match event {
            Event::Key(key) => key,
            Event::Resize(_, _) => {
                redraw = true;
                continue;
            }
            _ => continue,
        };
        match operation_for_key(reader_state.screen(), key) {
            ReaderOperation::Quit => {
                if reader_state.screen() == ReaderScreen::Article {
                    reader_state.close_article(state);
                    redraw = true;
                } else {
                    break;
                }
            }
            ReaderOperation::ToggleThread(letter) => {
                let (_, height) = terminal::size()?;
                let pages = thread_pages(&reader_state.threads, state, height);
                reader_state.normalize_page(pages.len());
                if let Some(page) = pages.get(reader_state.thread_page) {
                    reader_state.toggle_visible_thread(letter, page);
                }
                redraw = true;
            }
            ReaderOperation::NextPage => {
                let (_, height) = terminal::size()?;
                let pages = thread_pages(&reader_state.threads, state, height);
                let old_page = reader_state.thread_page;
                reader_state.next_page(pages.len());
                redraw = old_page != reader_state.thread_page;
            }
            ReaderOperation::PreviousPage => {
                let old_page = reader_state.thread_page;
                reader_state.previous_page();
                redraw = old_page != reader_state.thread_page;
            }
            ReaderOperation::StartSelectedThreads => {
                let (_, height) = terminal::size()?;
                let pages = thread_pages(&reader_state.threads, state, height);
                reader_state.normalize_page(pages.len());
                if reader_state.thread_page + 1 < pages.len() {
                    reader_state.next_page(pages.len());
                } else {
                    reader_state
                        .start_reading(state)
                        .map_err(ReaderError::State)?;
                }
                redraw = true;
            }
            ReaderOperation::CloseArticle => {
                reader_state.close_article(state);
                redraw = true;
            }
            ReaderOperation::NextArticle => {
                if reader_state.next_article().is_none()
                    && !reader_state
                        .advance_to_next_thread(state)
                        .map_err(ReaderError::State)?
                {
                    reader_state.close_article(state);
                }
                redraw = true;
            }
            ReaderOperation::Redraw => redraw = true,
            ReaderOperation::Ignore => {}
        }
    }
    Ok(())
}

fn fetch_decoded_article(
    session: &mut NntpSession,
    article_number: u64,
) -> Result<decoder::DecodedArticle, ReaderError> {
    let headers = session
        .article_headers(article_number)
        .map_err(ReaderError::Article)?;
    let body = session
        .article_body(article_number)
        .map_err(ReaderError::Article)?;
    let mut lines = headers;
    lines.push(Vec::new());
    lines.extend(body);
    let parsed = crate::model::article::RawArticle::parse(&lines);
    let decoded = decoder::decode_article(&parsed.article);
    Ok(decoded)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ThreadEntry {
    index: usize,
    authors: Vec<String>,
    omitted_authors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ThreadPage {
    entries: Vec<ThreadEntry>,
    end_index: usize,
}

fn thread_pages(threads: &ThreadList, state: &SessionState, height: u16) -> Vec<ThreadPage> {
    let mut pages = Vec::new();
    let capacity = usize::from(height).saturating_sub(2);
    if capacity == 0 {
        return vec![ThreadPage::default()];
    }
    let mut page = ThreadPage::default();
    let mut used = 0;
    for (index, row) in threads.rows.iter().enumerate() {
        if page.entries.len() == 25 {
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

fn thread_authors(row: &crate::state::thread_list::ThreadRow, state: &SessionState) -> Vec<String> {
    let mut article_numbers = row.article_numbers.clone();
    if let Some(first_unread) = article_numbers
        .iter()
        .position(|number| !state.is_read(&row.group_name, *number))
    {
        article_numbers.rotate_left(first_unread);
    }
    article_numbers
        .iter()
        .map(|number| {
            state
                .groups
                .get(&row.group_name)
                .and_then(|group| group.articles.get(number))
                .and_then(|metadata| metadata.from.as_deref())
                .map(|from| crate::decoder::decode_header_value(b"From", from).0)
                .unwrap_or_else(|| "(unknown author)".to_owned())
        })
        .collect()
}

fn thread_letter(index: usize) -> Option<char> {
    "abcdefghijklmnoprstuvwxyz".chars().nth(index)
}

fn assigned_thread_letter(
    page: &ThreadPage,
    threads: &ThreadList,
    row_index: usize,
) -> Option<char> {
    let mut letter_index = 0;
    for entry in &page.entries {
        if threads.rows.get(entry.index)?.unread_count > 0 {
            if entry.index == row_index {
                return thread_letter(letter_index);
            }
            letter_index += 1;
        }
    }
    None
}

fn page_label(index: usize, count: usize, end_index: usize, total: usize) -> String {
    if count <= 1 {
        return "All".to_owned();
    }
    if index + 1 == count {
        return "Bot".to_owned();
    }
    let percent = if total == 0 {
        100
    } else {
        end_index.saturating_mul(100).div_ceil(total)
    };
    if index == 0 {
        format!("Top {percent}%")
    } else {
        format!("{percent}%")
    }
}

fn render_thread_list(
    output: &mut impl Write,
    reader: &ReaderState,
    state: &SessionState,
    pages: &[ThreadPage],
    width: u16,
    height: u16,
) -> io::Result<()> {
    if height == 0 {
        return output.flush();
    }
    let page_index = reader.thread_page.min(pages.len().saturating_sub(1));
    let page = pages.get(page_index).cloned().unwrap_or_default();
    let page_position = page_label(
        page_index,
        pages.len(),
        page.end_index,
        reader.threads.rows.len(),
    );
    let article_count = reader
        .threads
        .rows
        .iter()
        .map(|row| row.article_numbers.len())
        .sum::<usize>();
    queue!(
        output,
        Clear(ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    )?;
    write_terminal_line(
        output,
        format_args!(
            "{}          {} articles",
            reader.threads.group_name, article_count
        ),
        height >= 2,
        width,
    )?;
    if height < 3 {
        if height == 2 {
            write_terminal_line(output, format_args!("-- {page_position} --"), false, width)?;
        }
        return output.flush();
    }
    if reader.threads.rows.is_empty() {
        write_terminal_line(output, format_args!("No threads"), true, width)?;
    }
    for entry in &page.entries {
        let row = &reader.threads.rows[entry.index];
        let letter = assigned_thread_letter(&page, &reader.threads, entry.index).unwrap_or(' ');
        let marked = reader.marked_threads.contains(&row.root_article_number);
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
            .map(|subject| crate::decoder::decode_header_value(b"Subject", subject).0)
            .unwrap_or_else(|| row.subject.clone());
        let line = format!(
            "{}{} {:<20} {:>3}  >{}{}",
            letter,
            if marked && letter != ' ' { "+" } else { " " },
            author,
            row.unread_count,
            lead,
            unresolved
        );
        write_terminal_line(output, format_args!("{line}"), true, width)?;
        for author in entry.authors.iter().skip(1) {
            write_terminal_line(output, format_args!("  {author}"), true, width)?;
        }
        if entry.omitted_authors > 0 {
            write_terminal_line(
                output,
                format_args!("  ... etc. [{} lines total]", entry.omitted_authors),
                true,
                width,
            )?;
        }
    }
    write_terminal_line(
        output,
        format_args!(
            "-- Select threads (date order) -- {page_position} --  < > / Space: page  q: quit"
        ),
        false,
        width,
    )?;
    output.flush()
}

fn write_terminal_line(
    output: &mut impl Write,
    arguments: fmt::Arguments<'_>,
    terminate: bool,
    width: u16,
) -> io::Result<()> {
    let mut line = String::new();
    fmt::write(&mut line, arguments)
        .map_err(|_| io::Error::other("could not format terminal line"))?;
    output.write_all(fit_terminal_line(&line, width).as_bytes())?;
    if terminate {
        output.write_all(b"\r\n")?;
    }
    Ok(())
}

fn fit_terminal_line(line: &str, width: u16) -> String {
    let sanitized: String = line
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect();
    let width = usize::from(width);
    if sanitized.len() <= width {
        return sanitized;
    }

    let mut fitted = String::new();
    let prefix_limit = width.saturating_sub('…'.len_utf8());
    for character in sanitized.chars() {
        if fitted.len() + character.len_utf8() > prefix_limit {
            break;
        }
        fitted.push(character);
    }
    if width > 0 {
        fitted.push('…');
    }
    fitted
}

#[derive(Debug)]
pub enum ReaderError {
    Io(io::Error),
    Article(ArticleError),
    Present(crate::terminal::article_view::PresentArticleError),
    State(ReaderStateError),
}

impl std::fmt::Display for ReaderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "terminal I/O failed: {error}"),
            Self::Article(error) => write!(formatter, "article retrieval failed: {error}"),
            Self::Present(error) => write!(formatter, "{error}"),
            Self::State(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ReaderError {}

impl From<io::Error> for ReaderError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

struct TerminalSession {
    output: io::Stdout,
    raw_mode: bool,
    alternate_screen: bool,
}

impl TerminalSession {
    fn enter() -> io::Result<Self> {
        let mut output = io::stdout();
        execute!(output, EnterAlternateScreen)?;
        if let Err(error) = terminal::enable_raw_mode() {
            let _ = execute!(output, LeaveAlternateScreen);
            return Err(error);
        }
        Ok(Self {
            output,
            raw_mode: true,
            alternate_screen: true,
        })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if self.raw_mode {
            let _ = terminal::disable_raw_mode();
        }
        if self.alternate_screen {
            let _ = execute!(self.output, LeaveAlternateScreen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::article::{ArticleMetadata, MessageId};
    use crate::state::thread_list::ThreadList;
    use crate::state::LoadedArticle;
    use crossterm::event::KeyModifiers;

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

    fn reader_for(state: &SessionState) -> ReaderState {
        ReaderState::new(ThreadList::from_session(state, "comp.test").unwrap())
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn thread_pager_keys_are_literal_and_screen_specific() {
        assert_eq!(thread_letter(16), Some('r'));
        assert_eq!(thread_letter(25), None);
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char('j'))),
            ReaderOperation::ToggleThread('j')
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Article, key(KeyCode::Char('j'))),
            ReaderOperation::Ignore
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char('>'))),
            ReaderOperation::NextPage
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char('<'))),
            ReaderOperation::PreviousPage
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Right)),
            ReaderOperation::Ignore
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char('q'))),
            ReaderOperation::Quit
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Article, key(KeyCode::Char(' '))),
            ReaderOperation::NextArticle
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char(' '))),
            ReaderOperation::StartSelectedThreads
        );
    }

    #[test]
    fn marks_persist_across_pages_and_space_starts_marked_threads_on_last_page() {
        let data = session_with_threads(5);
        let mut reader = reader_for(&data);
        let pages = thread_pages(&reader.threads, &data, 4);
        assert_eq!(pages.len(), 3);

        reader.toggle_visible_thread('a', &pages[0]);
        assert!(reader.marked_threads.contains(&5));
        let mut output = Vec::new();
        render_thread_list(&mut output, &reader, &data, &pages, 80, 4).unwrap();
        assert!(String::from_utf8(output).unwrap().contains("a+"));
        reader.toggle_visible_thread('a', &pages[0]);
        assert!(!reader.marked_threads.contains(&5));
        reader.toggle_visible_thread('a', &pages[0]);
        assert!(reader.marked_threads.contains(&5));
        reader.next_page(pages.len());
        reader.toggle_visible_thread('b', &pages[1]);
        assert!(reader.marked_threads.contains(&2));
        reader.next_page(pages.len());
        reader.start_reading(&data).unwrap();

        assert_eq!(reader.screen(), ReaderScreen::Article);
        assert_eq!(reader.selected_article_number(), Some(5));
        assert_eq!(reader.reading_threads, [5, 2]);
        assert!(!data.is_read("comp.test", 5));
    }

    #[test]
    fn space_advances_until_last_page_then_unmarked_means_all_threads() {
        let data = session_with_threads(5);
        let mut reader = reader_for(&data);
        let pages = thread_pages(&reader.threads, &data, 4);
        reader.next_page(pages.len());
        assert_eq!(reader.thread_page, 1);
        reader.next_page(pages.len());
        assert_eq!(reader.thread_page, 2);
        reader.start_reading(&data).unwrap();
        assert_eq!(reader.reading_threads, [5, 4, 3, 2, 1]);
    }

    #[test]
    fn previous_and_next_page_clamp_at_ends_and_resize_clamps_page_index() {
        let data = session_with_threads(5);
        let mut reader = reader_for(&data);
        let small_pages = thread_pages(&reader.threads, &data, 4);
        reader.previous_page();
        assert_eq!(reader.thread_page, 0);
        reader.next_page(small_pages.len());
        reader.next_page(small_pages.len());
        reader.next_page(small_pages.len());
        assert_eq!(reader.thread_page, 2);

        let large_pages = thread_pages(&reader.threads, &data, 20);
        reader.normalize_page(large_pages.len());
        assert_eq!(reader.thread_page, 0);
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
        let pages = thread_pages(&threads, &data, 5);
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].entries[0].index, 0);
        assert_eq!(pages[0].entries[0].omitted_authors, 2);
        assert_eq!(pages[1].entries[0].index, 1);

        let mut reader = ReaderState::new(threads);
        reader.thread_page = 0;
        let mut output = Vec::new();
        render_thread_list(&mut output, &reader, &data, &pages, 80, 5).unwrap();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("... etc. [2 lines total]"));
        assert!(!rendered.contains("author-reply-3"));
        assert!(rendered.contains("Top 50%"));
        assert_eq!(rendered.matches("\r\n").count(), 4);
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
        let pages = thread_pages(&threads, &data, 4);

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].entries.len(), 1);
        assert_eq!(pages[0].entries[0].index, 0);
        assert_eq!(pages[0].entries[0].omitted_authors, 0);
        assert_eq!(pages[1].entries[0].index, 1);
    }

    #[test]
    fn empty_thread_list_does_not_enter_article_view() {
        let mut data = SessionState::default();
        data.load_articles("comp.test", std::iter::empty::<LoadedArticle>())
            .unwrap();
        let mut reader = reader_for(&data);

        reader.start_reading(&data).unwrap();

        assert_eq!(reader.screen(), ReaderScreen::Threads);
    }

    #[test]
    fn fully_read_threads_have_no_letter_assignment() {
        let mut data = session_with_threads(1);
        data.mark_article_read("comp.test", 1).unwrap();
        let reader = reader_for(&data);
        let pages = thread_pages(&reader.threads, &data, 5);
        let mut output = Vec::new();

        render_thread_list(&mut output, &reader, &data, &pages, 80, 5).unwrap();

        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("   author-thread-1"));
        assert_eq!(assigned_thread_letter(&pages[0], &reader.threads, 0), None);
    }

    #[test]
    fn terminal_line_is_sanitized_and_fitted_to_columns() {
        assert_eq!(fit_terminal_line("ab\u{1b}cdef", 5), "ab…");
        assert_eq!(fit_terminal_line("a\u{1b}b", 10), "a�b");
        assert_eq!(fit_terminal_line("abcdef", 1), "…");
        assert_eq!(fit_terminal_line("okay", 10), "okay");
    }

    #[test]
    fn reading_queue_advances_to_next_thread_and_returns_after_final_thread() {
        let data = session_with_threads(2);
        let mut reader = reader_for(&data);
        reader.marked_threads.insert(2);
        reader.marked_threads.insert(1);
        reader.start_reading(&data).unwrap();
        assert_eq!(reader.selected_article_number(), Some(2));
        assert!(reader.advance_to_next_thread(&data).unwrap());
        assert_eq!(reader.selected_article_number(), Some(1));
        assert!(!reader.advance_to_next_thread(&data).unwrap());
        assert_eq!(reader.screen(), ReaderScreen::Threads);
    }
}
