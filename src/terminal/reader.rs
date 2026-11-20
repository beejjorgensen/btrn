use crate::decoder;
use crate::nntp::{ArticleError, NntpSession};
use crate::state::article_navigation::ArticleNavigation;
use crate::state::thread_list::{ThreadList, ThreadSelectionError};
use crate::state::{SessionState, StateError};
use crate::terminal::article_view::present_article;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::style::{Attribute, SetAttribute};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, queue};
use std::fmt;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderOperation {
    Quit,
    MoveNext,
    MovePrevious,
    OpenSelected,
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
        (ReaderScreen::Threads, KeyCode::Down | KeyCode::Char('j')) => ReaderOperation::MoveNext,
        (ReaderScreen::Threads, KeyCode::Up | KeyCode::Char('k')) => ReaderOperation::MovePrevious,
        (ReaderScreen::Threads, KeyCode::Enter | KeyCode::Char(' ')) => {
            ReaderOperation::OpenSelected
        }
        (ReaderScreen::Article, KeyCode::Char(' ')) => ReaderOperation::NextArticle,
        (ReaderScreen::Article, KeyCode::Backspace) => ReaderOperation::CloseArticle,
        (ReaderScreen::Threads, KeyCode::Char('r')) => ReaderOperation::Redraw,
        (ReaderScreen::Article, KeyCode::Char('r')) => ReaderOperation::Redraw,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReaderStateError {
    State(StateError),
    Selection(ThreadSelectionError),
    NoThreadSelected,
    NoArticleSelected,
}

impl std::fmt::Display for ReaderStateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(error) => write!(formatter, "{error}"),
            Self::Selection(error) => write!(formatter, "{error}"),
            Self::NoThreadSelected => write!(formatter, "no thread is selected"),
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

    pub fn move_next_thread(&mut self) {
        self.threads.select_next();
    }

    pub fn move_previous_thread(&mut self) {
        self.threads.select_previous();
    }

    pub fn select_thread(&mut self, index: usize) -> Result<(), ReaderStateError> {
        self.threads
            .select(index)
            .map_err(ReaderStateError::Selection)
    }

    pub fn open_selected_thread(&mut self, state: &SessionState) -> Result<u64, ReaderStateError> {
        let row = self
            .threads
            .selected()
            .ok_or(ReaderStateError::NoThreadSelected)?;
        let navigation =
            ArticleNavigation::from_thread(state, &row.group_name, row.root_article_number)
                .map_err(ReaderStateError::State)?;
        let article_number = navigation
            .selected_article_number()
            .ok_or(ReaderStateError::NoArticleSelected)?;
        self.article_navigation = Some(navigation);
        Ok(article_number)
    }

    pub fn next_article(&mut self) -> Option<u64> {
        self.article_navigation.as_mut()?.next_article()
    }

    pub fn close_article(&mut self, state: &SessionState) {
        self.article_navigation = None;
        if let Ok(updated) = ThreadList::from_session(state, &self.threads.group_name) {
            let selected_index = self.threads.selected_index();
            self.threads = updated;
            if let Some(index) = selected_index.filter(|index| *index < self.threads.rows.len()) {
                let _ = self.threads.select(index);
            }
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
                ReaderScreen::Threads => render_thread_list(&mut terminal.output, &reader_state)?,
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
                    )?;
                    terminal.output.flush()?;
                }
            }
            redraw = false;
        }

        let Event::Key(key) = event::read()? else {
            continue;
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
            ReaderOperation::MoveNext => {
                reader_state.move_next_thread();
                redraw = true;
            }
            ReaderOperation::MovePrevious => {
                reader_state.move_previous_thread();
                redraw = true;
            }
            ReaderOperation::OpenSelected => {
                reader_state
                    .open_selected_thread(state)
                    .map_err(ReaderError::State)?;
                redraw = true;
            }
            ReaderOperation::CloseArticle => {
                reader_state.close_article(state);
                redraw = true;
            }
            ReaderOperation::NextArticle => {
                if reader_state.next_article().is_none() {
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

fn render_thread_list(output: &mut impl Write, state: &ReaderState) -> io::Result<()> {
    queue!(
        output,
        Clear(ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    )?;
    write_terminal_line(
        output,
        format_args!(
            "{} - {} threads",
            state.threads.group_name,
            state.threads.rows.len()
        ),
    )?;
    for (index, row) in state.threads.rows.iter().enumerate() {
        if state.threads.selected_index() == Some(index) {
            queue!(output, SetAttribute(Attribute::Reverse))?;
        }
        let unresolved = if row.has_unresolved_parent {
            " (?)"
        } else {
            ""
        };
        write_terminal_line(
            output,
            format_args!(
                "{:>3}/{:<3} {}{} {}",
                row.unread_count,
                row.article_numbers.len(),
                row.root_article_number,
                unresolved,
                row.subject
            ),
        )?;
        if state.threads.selected_index() == Some(index) {
            queue!(output, SetAttribute(Attribute::Reset))?;
        }
    }
    write_terminal_line(output, format_args!("j/k: select  enter: open  q: quit"))?;
    output.flush()
}

fn write_terminal_line(output: &mut impl Write, arguments: fmt::Arguments<'_>) -> io::Result<()> {
    output.write_fmt(arguments)?;
    output.write_all(b"\r\n")
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
                from: None,
                subject: Some(id.as_bytes().to_vec()),
                date: None,
                issues: Vec::new(),
            },
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn key_bindings_are_screen_specific() {
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char('j'))),
            ReaderOperation::MoveNext
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Article, key(KeyCode::Char('j'))),
            ReaderOperation::Ignore
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Article, key(KeyCode::Char(' '))),
            ReaderOperation::NextArticle
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char(' '))),
            ReaderOperation::OpenSelected
        );
    }

    #[test]
    fn transitions_open_thread_advance_and_return_without_marking_read() {
        let mut data = SessionState::default();
        data.load_articles(
            "comp.test",
            [
                loaded_article(10, "root", None),
                loaded_article(30, "reply", Some("root")),
                loaded_article(50, "other", None),
            ],
        )
        .unwrap();
        let threads = ThreadList::from_session(&data, "comp.test").unwrap();
        let mut reader = ReaderState::new(threads);
        reader.select_thread(1).unwrap();
        assert_eq!(reader.screen(), ReaderScreen::Threads);
        assert_eq!(reader.open_selected_thread(&data), Ok(10));
        assert_eq!(reader.screen(), ReaderScreen::Article);
        assert_eq!(reader.next_article(), Some(30));
        assert_eq!(reader.next_article(), None);
        assert!(!data.is_read("comp.test", 10));
        assert!(!data.is_read("comp.test", 30));
        reader.close_article(&data);
        assert_eq!(reader.screen(), ReaderScreen::Threads);
        assert_eq!(
            reader.threads.selected().map(|row| row.root_article_number),
            Some(10)
        );
    }

    #[test]
    fn thread_list_uses_crlf_line_endings() {
        let mut data = SessionState::default();
        data.load_articles("comp.test", [loaded_article(10, "root", None)])
            .unwrap();
        let threads = ThreadList::from_session(&data, "comp.test").unwrap();
        let reader = ReaderState::new(threads);
        let mut output = Vec::new();

        render_thread_list(&mut output, &reader).unwrap();

        assert!(output.windows(2).any(|bytes| bytes == b"\r\n"));
        assert!(!output
            .windows(2)
            .any(|bytes| bytes[1] == b'\n' && bytes[0] != b'\r'));
    }
}
