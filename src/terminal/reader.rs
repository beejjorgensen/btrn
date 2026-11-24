use crate::decoder;
use crate::model::article::{ArticleMetadata, MessageId, RawArticle};
use crate::nntp::{ArticleError, NntpSession};
use crate::posting::PostingSettings;
use crate::state::article_navigation::ArticleNavigation;
use crate::state::thread_hierarchy::{build_hierarchy, HierarchyGrid};
use crate::state::thread_list::ThreadList;
use crate::state::{LoadedArticle, SessionState, StateError};
use crate::terminal::article_view::{
    present_article, prompt_line, render_article_page, ArticleLayout, ArticleViewContext,
};
use crate::terminal::selector;
use crate::terminal::session::TerminalSession;
use crate::terminal::thread_selector::{
    assigned_thread_letter, page_thread_row_y, render_thread_list, thread_pages, ThreadPage,
};
use crate::thread::ThreadRef;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::terminal;
use std::collections::BTreeSet;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderOperation {
    Quit,
    FinishGroup,
    ToggleThread(char),
    NextPage,
    PreviousPage,
    StartSelectedThreads,
    CloseArticle,
    NextArticle,
    NextUnreadArticle,
    PreviousUnreadArticle,
    Followup,
    /// `[`, `]`, `(`, `)`, and the arrow keys: move through the thread
    /// window's graph, fetching a placeholder by message ID when the move
    /// lands on one.
    ThreadMove(ThreadMotion),
    Redraw,
    /// Ctrl-C: exit the program from any screen.
    Interrupt,
    Ignore,
}

pub fn operation_for_key(screen: ReaderScreen, key: KeyEvent) -> ReaderOperation {
    if key.kind != KeyEventKind::Press {
        return ReaderOperation::Ignore;
    }
    if crate::terminal::is_interrupt(&key) {
        return ReaderOperation::Interrupt;
    }
    match (screen, key.code) {
        (_, KeyCode::Esc) => ReaderOperation::Quit,
        (ReaderScreen::Threads, KeyCode::Char('q')) => ReaderOperation::FinishGroup,
        (ReaderScreen::Article, KeyCode::Char('q')) => ReaderOperation::Quit,
        (ReaderScreen::Threads, KeyCode::Char('>')) => ReaderOperation::NextPage,
        (ReaderScreen::Threads, KeyCode::Char('<')) => ReaderOperation::PreviousPage,
        (ReaderScreen::Threads, KeyCode::Char(' ')) => ReaderOperation::StartSelectedThreads,
        (ReaderScreen::Article, KeyCode::Char(' ')) => ReaderOperation::NextArticle,
        (ReaderScreen::Article, KeyCode::Char('n')) => ReaderOperation::NextUnreadArticle,
        (ReaderScreen::Article, KeyCode::Char('p')) => ReaderOperation::PreviousUnreadArticle,
        (ReaderScreen::Article, KeyCode::Backspace) => ReaderOperation::CloseArticle,
        (ReaderScreen::Article, KeyCode::Char('r')) => ReaderOperation::Redraw,
        (ReaderScreen::Article, KeyCode::Char('F')) => ReaderOperation::Followup,
        (ReaderScreen::Article, KeyCode::Char('[') | KeyCode::Left) => {
            ReaderOperation::ThreadMove(ThreadMotion::Parent)
        }
        (ReaderScreen::Article, KeyCode::Char(']') | KeyCode::Right) => {
            ReaderOperation::ThreadMove(ThreadMotion::FirstChild)
        }
        (ReaderScreen::Article, KeyCode::Char('(') | KeyCode::Up) => {
            ReaderOperation::ThreadMove(ThreadMotion::PreviousSibling)
        }
        (ReaderScreen::Article, KeyCode::Char(')') | KeyCode::Down) => {
            ReaderOperation::ThreadMove(ThreadMotion::NextSibling)
        }
        (ReaderScreen::Threads, KeyCode::Char(letter))
            if letter.is_ascii_alphanumeric() && letter != 'q' =>
        {
            ReaderOperation::ToggleThread(letter.to_ascii_lowercase())
        }
        _ => ReaderOperation::Ignore,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReaderExit {
    FinishGroup,
    QuitApplication,
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
    group_last_article: u64,
    article_page: usize,
    current_article: Option<CurrentArticle>,
    /// The article viewed before the current one, highlighted in the thread
    /// window.
    previous_article: Option<u64>,
    /// A one-time message shown after the article prompt on the next redraw.
    notice: Option<&'static str>,
}

/// A move through the thread window's graph from the current article.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadMotion {
    /// `[` or Left: the parent, which may be on an earlier row.
    Parent,
    /// `]` or Right: the first reply, on the same row.
    FirstChild,
    /// `(` or Up: the nearest node drawn straight above.
    PreviousSibling,
    /// `)` or Down: the nearest node drawn straight below.
    NextSibling,
}

impl ThreadMotion {
    /// The notice shown when the move would leave the graph.
    fn off_graph_notice(self) -> &'static str {
        match self {
            Self::Parent => "There is no parent article prior to this one.",
            Self::FirstChild => "This is the last leaf in this tree.",
            Self::PreviousSibling => "This article has no previous sibling.",
            Self::NextSibling => "This article has no next sibling.",
        }
    }

    /// The notice shown when the target placeholder cannot be fetched.
    fn unavailable_notice(self) -> &'static str {
        match self {
            Self::Parent => "Parent article is not available.",
            _ => "Article is not available.",
        }
    }
}

/// Where a [`ThreadMotion`] leads from the current article.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MoveTarget {
    /// The move leaves the graph.
    None,
    Loaded(u64),
    /// The target is a placeholder for an article that is not loaded.
    Missing(MessageId),
}

impl From<Option<ThreadRef>> for MoveTarget {
    fn from(node: Option<ThreadRef>) -> Self {
        match node {
            None => Self::None,
            Some(ThreadRef::Article(number)) => Self::Loaded(number),
            Some(ThreadRef::Missing(id)) => Self::Missing(id),
        }
    }
}

/// The decoded article on screen, kept in memory only so paging and redraws do
/// not refetch it or mark it read again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentArticle {
    article_number: u64,
    article: decoder::DecodedArticle,
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
    pub fn new(mut threads: ThreadList, group_last_article: u64) -> Self {
        threads.rows.retain(|row| row.unread_count > 0);
        Self {
            threads,
            article_navigation: None,
            thread_page: 0,
            marked_threads: BTreeSet::new(),
            reading_threads: Vec::new(),
            reading_thread_index: 0,
            group_last_article,
            article_page: 0,
            current_article: None,
            previous_article: None,
            notice: None,
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

    fn toggle_visible_thread(&mut self, letter: char, page: &ThreadPage) -> Option<(usize, bool)> {
        let index = page.entries.iter().find_map(|entry| {
            let row = self.threads.rows.get(entry.index)?;
            (row.unread_count > 0
                && assigned_thread_letter(page, &self.threads, entry.index) == Some(letter))
            .then_some(entry.index)
        })?;
        let row = self.threads.rows.get(index)?;
        let root_article_number = row.root_article_number;
        self.toggle_thread(root_article_number);
        Some((index, self.marked_threads.contains(&root_article_number)))
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

    fn should_start_on_space(&self, page_count: usize) -> bool {
        !self.marked_threads.is_empty() || self.thread_page + 1 >= page_count
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

    pub(crate) fn move_target(&self, state: &SessionState, motion: ThreadMotion) -> MoveTarget {
        let Some(article_number) = self.selected_article_number() else {
            return MoveTarget::None;
        };
        let Some(group) = state.groups.get(&self.threads.group_name) else {
            return MoveTarget::None;
        };
        let current = ThreadRef::Article(article_number);
        let node = match motion {
            ThreadMotion::Parent => group.threads.parent(&current),
            ThreadMotion::FirstChild => group.threads.children(&current).first().cloned(),
            ThreadMotion::PreviousSibling => self
                .hierarchy(state, article_number)
                .and_then(|grid| grid.cell_above()),
            ThreadMotion::NextSibling => self
                .hierarchy(state, article_number)
                .and_then(|grid| grid.cell_below()),
        };
        node.into()
    }

    /// The thread window's graph with `article_number` current.
    fn hierarchy(&self, state: &SessionState, article_number: u64) -> Option<HierarchyGrid> {
        let root = self
            .article_navigation
            .as_ref()
            .and_then(ArticleNavigation::root_article_number)?;
        build_hierarchy(
            state,
            &self.threads.group_name,
            root,
            article_number,
            self.previous_article,
        )
        .ok()
    }

    pub(crate) fn select_article(&mut self, article_number: u64) -> bool {
        self.article_navigation
            .as_mut()
            .is_some_and(|navigation| navigation.select_article(article_number))
    }

    /// Adds an article fetched by message ID to the group and selects it. The
    /// thread is rebuilt because the new article can replace placeholders
    /// and join the current thread to others. Loading it changes no read
    /// state.
    pub(crate) fn adopt_fetched_article(
        &mut self,
        state: &mut SessionState,
        article_number: u64,
        metadata: ArticleMetadata,
    ) -> Result<(), ReaderStateError> {
        let group = self.threads.group_name.clone();
        state
            .load_articles(
                &group,
                [LoadedArticle {
                    article_number,
                    metadata,
                }],
            )
            .map_err(ReaderStateError::State)?;
        let first = state
            .groups
            .get(&group)
            .and_then(|loaded| loaded.threads.thread_first_article(article_number))
            .ok_or(ReaderStateError::NoArticleSelected)?;
        let mut navigation = ArticleNavigation::from_thread(state, &group, first)
            .map_err(ReaderStateError::State)?;
        navigation.select_article(article_number);
        self.article_navigation = Some(navigation);
        if let Some(root) = self.reading_threads.get_mut(self.reading_thread_index) {
            *root = first;
        }
        Ok(())
    }

    fn next_unread_article(&mut self, state: &SessionState) -> Option<u64> {
        self.article_navigation.as_mut()?.next_unread_article(state)
    }

    fn previous_unread_article(&mut self, state: &SessionState) -> Option<u64> {
        self.article_navigation
            .as_mut()?
            .previous_unread_article(state)
    }

    /// Moves to the next unread article in the reading run, returning to the
    /// thread selector after the last thread.
    fn advance_article(&mut self, state: &SessionState) -> Result<(), ReaderStateError> {
        if self.next_unread_article(state).is_none() && !self.advance_to_next_thread(state)? {
            self.close_article(state);
        }
        Ok(())
    }

    /// Advances within the current article's pages. Returns false when already
    /// on the last page.
    fn next_article_page(&mut self, page_count: usize) -> bool {
        if self.article_page + 1 < page_count {
            self.article_page += 1;
            true
        } else {
            false
        }
    }

    /// Unread articles in the group other than `article_number`.
    fn more_unread(&self, state: &SessionState, article_number: u64) -> usize {
        self.threads
            .rows
            .iter()
            .flat_map(|row| &row.article_numbers)
            .filter(|number| {
                **number != article_number && !state.is_read(&self.threads.group_name, **number)
            })
            .count()
    }

    fn cached_article(&self, article_number: u64) -> Option<&decoder::DecodedArticle> {
        self.current_article
            .as_ref()
            .filter(|current| current.article_number == article_number)
            .map(|current| &current.article)
    }

    /// The context and layout for the cached article, or `None` when
    /// `article_number` is not the cached article.
    fn article_view(
        &self,
        state: &SessionState,
        article_number: u64,
        width: u16,
    ) -> Option<(ArticleViewContext<'_>, ArticleLayout)> {
        let article = self.cached_article(article_number)?;
        let context = ArticleViewContext {
            group: &self.threads.group_name,
            article_number,
            more_unread: self.more_unread(state, article_number),
            group_last_article: self.group_last_article,
        };
        let hierarchy = self.hierarchy(state, article_number);
        let layout = match &hierarchy {
            Some(hierarchy) => ArticleLayout::with_hierarchy(&context, article, hierarchy, width),
            None => ArticleLayout::new(&context, article, width),
        };
        Some((context, layout))
    }

    /// Moves the article view to its last page; rendering clamps the index.
    fn show_last_article_page(&mut self) {
        self.article_page = usize::MAX;
    }

    fn set_current_article(&mut self, article_number: u64, article: decoder::DecodedArticle) {
        if let Some(current) = &self.current_article {
            if current.article_number != article_number {
                self.previous_article = Some(current.article_number);
            }
        }
        self.current_article = Some(CurrentArticle {
            article_number,
            article,
        });
        self.article_page = 0;
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
        self.current_article = None;
        self.previous_article = None;
        self.article_page = 0;
        self.reading_threads.clear();
        self.reading_thread_index = 0;
        if let Ok(mut updated) = ThreadList::from_session(state, &self.threads.group_name) {
            updated.rows.retain(|row| row.unread_count > 0);
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

pub(crate) fn run_interactive(
    session: &mut NntpSession,
    state: &mut SessionState,
    reader_state: ReaderState,
    posting: &PostingSettings,
) -> Result<ReaderExit, ReaderError> {
    let mut terminal = TerminalSession::enter()?;
    run_interactive_in_session(session, state, reader_state, &mut terminal, posting)
}

pub(crate) fn run_interactive_in_session(
    session: &mut NntpSession,
    state: &mut SessionState,
    mut reader_state: ReaderState,
    terminal: &mut TerminalSession,
    posting: &PostingSettings,
) -> Result<ReaderExit, ReaderError> {
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
                        &reader_state.threads,
                        &reader_state.marked_threads,
                        reader_state.thread_page,
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
                    let (width, height) = terminal::size()?;
                    let fresh = reader_state.cached_article(article_number).is_none();
                    if fresh {
                        let article = fetch_decoded_article(session, article_number)?;
                        reader_state.set_current_article(article_number, article);
                    }
                    let notice = reader_state.notice.take();
                    let (context, layout) = reader_state
                        .article_view(state, article_number, width)
                        .expect("current article was cached");
                    let page = reader_state.article_page.min(layout.page_count(height) - 1);
                    if fresh {
                        present_article(
                            &mut terminal.output,
                            state,
                            &context,
                            &layout,
                            width,
                            height,
                        )
                        .map_err(ReaderError::Present)?;
                    } else {
                        render_article_page(
                            &mut terminal.output,
                            &context,
                            &layout,
                            page,
                            width,
                            height,
                        )?;
                    }
                    if let Some(notice) = notice {
                        let prompt = prompt_line(&context, &layout, page, height);
                        let used = u16::try_from(prompt.chars().count()).unwrap_or(u16::MAX);
                        selector::write_line(
                            &mut terminal.output,
                            &format!("  {notice}"),
                            false,
                            width.saturating_sub(used),
                            false,
                        )?;
                        terminal.output.flush()?;
                    }
                    reader_state.article_page = page;
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
                    return Ok(ReaderExit::QuitApplication);
                }
            }
            ReaderOperation::FinishGroup => {
                if reader_state.screen() == ReaderScreen::Article {
                    reader_state.close_article(state);
                    redraw = true;
                } else {
                    return Ok(ReaderExit::FinishGroup);
                }
            }
            ReaderOperation::ToggleThread(letter) => {
                let (_, height) = terminal::size()?;
                let pages = thread_pages(&reader_state.threads, state, height);
                reader_state.normalize_page(pages.len());
                let updated = pages.get(reader_state.thread_page).and_then(|page| {
                    reader_state
                        .toggle_visible_thread(letter, page)
                        .map(|(row_index, marked)| (page_thread_row_y(page, row_index), marked))
                });
                if let Some((Some(row), marked)) = updated {
                    selector::write_mark(&mut terminal.output, 1, row, marked)?;
                } else if updated.is_some() {
                    redraw = true;
                }
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
                if reader_state.should_start_on_space(pages.len()) {
                    reader_state
                        .start_reading(state)
                        .map_err(ReaderError::State)?;
                } else {
                    reader_state.next_page(pages.len());
                }
                redraw = true;
            }
            ReaderOperation::CloseArticle => {
                reader_state.close_article(state);
                redraw = true;
            }
            ReaderOperation::NextArticle => {
                let (width, height) = terminal::size()?;
                let page_count = reader_state
                    .selected_article_number()
                    .and_then(|number| reader_state.article_view(state, number, width))
                    .map_or(1, |(_, layout)| layout.page_count(height));
                if !reader_state.next_article_page(page_count) {
                    reader_state
                        .advance_article(state)
                        .map_err(ReaderError::State)?;
                }
                redraw = true;
            }
            ReaderOperation::NextUnreadArticle => {
                reader_state
                    .advance_article(state)
                    .map_err(ReaderError::State)?;
                redraw = true;
            }
            ReaderOperation::PreviousUnreadArticle => {
                redraw = reader_state.previous_unread_article(state).is_some();
            }
            ReaderOperation::Followup => {
                let Some(article_number) = reader_state.selected_article_number() else {
                    continue;
                };
                let Some(article) = reader_state.cached_article(article_number).cloned() else {
                    continue;
                };
                let group = reader_state.threads.group_name.clone();
                match crate::terminal::posting::run_followup(
                    session, terminal, posting, &group, &article,
                )? {
                    crate::terminal::posting::PostingExit::QuitApplication => {
                        return Ok(ReaderExit::QuitApplication)
                    }
                    crate::terminal::posting::PostingExit::ReturnToArticle => {
                        reader_state.show_last_article_page();
                        redraw = true;
                    }
                }
            }
            ReaderOperation::ThreadMove(motion) => match reader_state.move_target(state, motion) {
                MoveTarget::None => {
                    reader_state.notice = Some(motion.off_graph_notice());
                    redraw = true;
                }
                MoveTarget::Loaded(number) => redraw = reader_state.select_article(number),
                MoveTarget::Missing(id) => {
                    let group = reader_state.threads.group_name.clone();
                    match fetch_metadata_by_id(session, &group, &id)? {
                        Some((number, metadata)) => reader_state
                            .adopt_fetched_article(state, number, metadata)
                            .map_err(ReaderError::State)?,
                        None => reader_state.notice = Some(motion.unavailable_notice()),
                    }
                    redraw = true;
                }
            },
            ReaderOperation::Interrupt => return Ok(ReaderExit::QuitApplication),
            ReaderOperation::Redraw => redraw = true,
            ReaderOperation::Ignore => {}
        }
    }
}

/// The group article number and metadata of the article with `id`, or
/// `None` when the server no longer has it or it is not in `group`.
fn fetch_metadata_by_id(
    session: &mut NntpSession,
    group: &str,
    id: &MessageId,
) -> Result<Option<(u64, ArticleMetadata)>, ReaderError> {
    let mut lines = match session.article_headers_by_id(id) {
        Ok(lines) => lines,
        Err(ArticleError::ArticleNotFound { .. }) => return Ok(None),
        Err(error) => return Err(ReaderError::Article(error)),
    };
    lines.push(Vec::new());
    let article = RawArticle::parse(&lines).article;
    Ok(article
        .xref_article_number(group)
        .map(|number| (number, article.metadata())))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::thread_list::ThreadList;
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
        ReaderState::new(ThreadList::from_session(state, "comp.test").unwrap(), 100)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn thread_pager_keys_are_literal_and_screen_specific() {
        assert_eq!(selector::selection_letter(16), Some('r'));
        assert_eq!(selector::selection_letter(24), Some('z'));
        assert_eq!(selector::selection_letter(25), Some('0'));
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char('j'))),
            ReaderOperation::ToggleThread('j')
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char('0'))),
            ReaderOperation::ToggleThread('0')
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
            ReaderOperation::FinishGroup
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Esc)),
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
        assert_eq!(
            operation_for_key(ReaderScreen::Article, key(KeyCode::Char('n'))),
            ReaderOperation::NextUnreadArticle
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Article, key(KeyCode::Char('p'))),
            ReaderOperation::PreviousUnreadArticle
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char('n'))),
            ReaderOperation::ToggleThread('n')
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Article, key(KeyCode::Char('F'))),
            ReaderOperation::Followup
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char('F'))),
            ReaderOperation::ToggleThread('f')
        );
        let control_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        for screen in [ReaderScreen::Threads, ReaderScreen::Article] {
            assert_eq!(
                operation_for_key(screen, control_c),
                ReaderOperation::Interrupt
            );
        }
    }

    fn decoded_body(body: &str) -> decoder::DecodedArticle {
        decoder::DecodedArticle {
            headers: Vec::new(),
            body: body.to_owned(),
            issues: Vec::new(),
        }
    }

    fn thread_session() -> SessionState {
        let mut data = SessionState::default();
        data.load_articles(
            "comp.test",
            [
                loaded_article(10, "root", None),
                loaded_article(11, "reply-1", Some("root")),
                loaded_article(12, "reply-2", Some("reply-1")),
                loaded_article(1, "other", None),
            ],
        )
        .unwrap();
        data
    }

    #[test]
    fn space_pages_through_the_article_before_advancing() {
        let data = thread_session();
        let mut reader = reader_for(&data);
        reader.marked_threads.insert(10);
        reader.start_reading(&data).unwrap();
        assert_eq!(reader.selected_article_number(), Some(10));
        reader.set_current_article(10, decoded_body("a\nb\nc"));

        assert!(reader.next_article_page(3));
        assert_eq!(reader.article_page, 1);
        assert!(reader.next_article_page(3));
        assert!(!reader.next_article_page(3));
        assert_eq!(reader.article_page, 2);
        assert_eq!(reader.selected_article_number(), Some(10));

        reader.advance_article(&data).unwrap();
        assert_eq!(reader.selected_article_number(), Some(11));
        assert_eq!(reader.cached_article(11), None);
        reader.set_current_article(11, decoded_body("x"));
        assert_eq!(reader.article_page, 0);
    }

    #[test]
    fn article_view_shows_the_thread_window_with_current_and_previous_articles() {
        let data = thread_session();
        let mut reader = reader_for(&data);
        reader.marked_threads.insert(10);
        reader.start_reading(&data).unwrap();
        reader.set_current_article(10, decoded_body("a"));
        assert_eq!(reader.previous_article, None);
        reader.advance_article(&data).unwrap();
        reader.set_current_article(11, decoded_body("b"));
        assert_eq!(reader.previous_article, Some(10));
        // Redrawing the same article keeps the previous article.
        reader.set_current_article(11, decoded_body("b"));
        assert_eq!(reader.previous_article, Some(10));

        // Each fixture article has its own subject, so they number 1, 2, 3.
        let (context, layout) = reader.article_view(&data, 11, 80).unwrap();
        let mut output = Vec::new();
        render_article_page(&mut output, &context, &layout, 0, 80, 24).unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("(\u{1b}[7m1\u{1b}[0m)--\u{1b}[7m[2]\u{1b}[0m--(3)"));

        reader.close_article(&data);
        assert_eq!(reader.previous_article, None);
    }

    #[test]
    fn next_and_previous_unread_skip_read_articles_and_leave_read_state_alone() {
        let mut data = thread_session();
        let mut reader = reader_for(&data);
        reader.marked_threads.insert(10);
        reader.start_reading(&data).unwrap();
        data.mark_article_read("comp.test", 10).unwrap();

        reader.advance_article(&data).unwrap();
        assert_eq!(reader.selected_article_number(), Some(11));
        assert_eq!(reader.previous_unread_article(&data), None);
        assert_eq!(reader.selected_article_number(), Some(11));
        reader.advance_article(&data).unwrap();
        assert_eq!(reader.selected_article_number(), Some(12));
        assert_eq!(reader.previous_unread_article(&data), Some(11));
        assert!(!data.is_read("comp.test", 11));
        assert!(!data.is_read("comp.test", 12));

        reader.advance_article(&data).unwrap();
        reader.advance_article(&data).unwrap();
        assert_eq!(reader.screen(), ReaderScreen::Threads);
        assert!(reader.current_article.is_none());
    }

    #[test]
    fn more_unread_excludes_the_current_and_already_read_articles() {
        let mut data = thread_session();
        let reader = reader_for(&data);
        assert_eq!(reader.more_unread(&data, 10), 3);
        data.mark_article_read("comp.test", 10).unwrap();
        data.mark_article_read("comp.test", 1).unwrap();
        assert_eq!(reader.more_unread(&data, 10), 2);
        assert_eq!(reader.more_unread(&data, 11), 1);
    }

    #[test]
    fn marks_persist_across_pages_and_space_starts_marked_threads_on_last_page() {
        let data = session_with_threads(5);
        let mut reader = reader_for(&data);
        let pages = thread_pages(&reader.threads, &data, 6);
        assert_eq!(pages.len(), 3);

        assert_eq!(
            reader.toggle_visible_thread('a', &pages[0]),
            Some((0, true))
        );
        assert!(reader.marked_threads.contains(&5));
        assert!(reader.should_start_on_space(pages.len()));
        let mut output = Vec::new();
        render_thread_list(
            &mut output,
            &reader.threads,
            &reader.marked_threads,
            reader.thread_page,
            &data,
            &pages,
            80,
            6,
        )
        .unwrap();
        let rendered = String::from_utf8(output).unwrap();
        assert!(rendered.contains("a+"));
        assert!(rendered.contains("\u{1b}[7mcomp.test\u{1b}[0m"));
        assert!(rendered.contains("\u{1b}[7m-- Select Threads"));
        assert_eq!(
            reader.toggle_visible_thread('a', &pages[0]),
            Some((0, false))
        );
        assert!(!reader.marked_threads.contains(&5));
        assert_eq!(
            reader.toggle_visible_thread('a', &pages[0]),
            Some((0, true))
        );
        assert!(reader.marked_threads.contains(&5));
        reader.next_page(pages.len());
        assert_eq!(
            reader.toggle_visible_thread('b', &pages[1]),
            Some((3, true))
        );
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
        let pages = thread_pages(&reader.threads, &data, 6);
        assert!(!reader.should_start_on_space(pages.len()));
        reader.next_page(pages.len());
        assert_eq!(reader.thread_page, 1);
        assert!(!reader.should_start_on_space(pages.len()));
        reader.next_page(pages.len());
        assert_eq!(reader.thread_page, 2);
        assert!(reader.should_start_on_space(pages.len()));
        reader.start_reading(&data).unwrap();
        assert_eq!(reader.reading_threads, [5, 4, 3, 2, 1]);
    }

    #[test]
    fn previous_and_next_page_clamp_at_ends_and_resize_clamps_page_index() {
        let data = session_with_threads(5);
        let mut reader = reader_for(&data);
        let small_pages = thread_pages(&reader.threads, &data, 6);
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
    fn thread_selector_hides_read_articles_and_skips_them_when_advancing() {
        let mut data = SessionState::default();
        data.load_articles(
            "comp.test",
            [
                loaded_article(10, "root", None),
                loaded_article(11, "read-reply", Some("root")),
                loaded_article(12, "unread-reply", Some("read-reply")),
            ],
        )
        .unwrap();
        data.mark_article_read("comp.test", 11).unwrap();
        let mut reader = reader_for(&data);
        let pages = thread_pages(&reader.threads, &data, 10);

        assert_eq!(
            pages[0].entries[0].authors,
            ["author-root", "author-unread-reply"]
        );
        reader.start_reading(&data).unwrap();
        assert_eq!(reader.selected_article_number(), Some(10));
        assert_eq!(reader.next_unread_article(&data), Some(12));
        assert_eq!(reader.next_unread_article(&data), None);
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

    #[test]
    fn thread_motion_keys_map_only_on_the_article_view() {
        let bindings = [
            (KeyCode::Char('['), ThreadMotion::Parent),
            (KeyCode::Left, ThreadMotion::Parent),
            (KeyCode::Char(']'), ThreadMotion::FirstChild),
            (KeyCode::Right, ThreadMotion::FirstChild),
            (KeyCode::Char('('), ThreadMotion::PreviousSibling),
            (KeyCode::Up, ThreadMotion::PreviousSibling),
            (KeyCode::Char(')'), ThreadMotion::NextSibling),
            (KeyCode::Down, ThreadMotion::NextSibling),
        ];
        for (code, motion) in bindings {
            assert_eq!(
                operation_for_key(ReaderScreen::Article, key(code)),
                ReaderOperation::ThreadMove(motion)
            );
            assert_eq!(
                operation_for_key(ReaderScreen::Threads, key(code)),
                ReaderOperation::Ignore
            );
        }
        // Shifted punctuation arrives with the Shift modifier.
        assert_eq!(
            operation_for_key(
                ReaderScreen::Article,
                KeyEvent::new(KeyCode::Char('('), KeyModifiers::SHIFT)
            ),
            ReaderOperation::ThreadMove(ThreadMotion::PreviousSibling)
        );
    }

    #[test]
    fn thread_motions_follow_the_graph_and_leave_read_state_alone() {
        // [10]+-(11)+-(12)
        //     |     \-(13)
        //     \-(14)
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [
                    loaded_article(10, "root", None),
                    loaded_article(11, "a", Some("root")),
                    loaded_article(12, "b", Some("a")),
                    loaded_article(13, "c", Some("a")),
                    loaded_article(14, "d", Some("root")),
                ],
            )
            .unwrap();
        let mut reader = reader_for(&state);
        reader.start_reading(&state).unwrap();
        let target = |reader: &ReaderState, motion| reader.move_target(&state, motion);

        assert_eq!(reader.selected_article_number(), Some(10));
        assert_eq!(target(&reader, ThreadMotion::Parent), MoveTarget::None);
        assert_eq!(
            target(&reader, ThreadMotion::FirstChild),
            MoveTarget::Loaded(11)
        );
        assert_eq!(
            target(&reader, ThreadMotion::PreviousSibling),
            MoveTarget::None
        );
        assert_eq!(target(&reader, ThreadMotion::NextSibling), MoveTarget::None);

        assert!(reader.select_article(12));
        assert_eq!(
            target(&reader, ThreadMotion::Parent),
            MoveTarget::Loaded(11)
        );
        assert_eq!(target(&reader, ThreadMotion::FirstChild), MoveTarget::None);
        assert_eq!(
            target(&reader, ThreadMotion::NextSibling),
            MoveTarget::Loaded(13)
        );

        assert!(reader.select_article(13));
        assert_eq!(
            target(&reader, ThreadMotion::PreviousSibling),
            MoveTarget::Loaded(12)
        );
        assert_eq!(target(&reader, ThreadMotion::NextSibling), MoveTarget::None);
        // Left from a later reply climbs to its parent's row.
        assert_eq!(
            target(&reader, ThreadMotion::Parent),
            MoveTarget::Loaded(11)
        );

        assert!(reader.select_article(14));
        assert_eq!(
            target(&reader, ThreadMotion::PreviousSibling),
            MoveTarget::Loaded(11)
        );
        assert_eq!(
            target(&reader, ThreadMotion::Parent),
            MoveTarget::Loaded(10)
        );

        assert!((10..=14).all(|number| !state.is_read("comp.test", number)));
        assert!(state.newsrc.entry("comp.test").is_none());
    }

    #[test]
    fn thread_motions_can_land_on_a_placeholder() {
        // 12 replies to <gone>, a missing reply to 10.
        let mut late = loaded_article(12, "late", Some("gone"));
        late.metadata
            .references
            .insert(0, MessageId::parse(b"<root>").unwrap());
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [
                    loaded_article(10, "root", None),
                    loaded_article(11, "a", Some("root")),
                    late,
                ],
            )
            .unwrap();
        let mut reader = reader_for(&state);
        reader.start_reading(&state).unwrap();
        let gone = MoveTarget::Missing(MessageId::parse(b"<gone>").unwrap());
        assert!(reader.select_article(11));
        assert_eq!(reader.move_target(&state, ThreadMotion::NextSibling), gone);
        assert!(reader.select_article(12));
        assert_eq!(reader.move_target(&state, ThreadMotion::Parent), gone);
    }

    #[test]
    fn off_graph_notices_name_the_direction() {
        assert_eq!(
            ThreadMotion::Parent.off_graph_notice(),
            "There is no parent article prior to this one."
        );
        assert_eq!(
            ThreadMotion::FirstChild.off_graph_notice(),
            "This is the last leaf in this tree."
        );
        assert_eq!(
            ThreadMotion::PreviousSibling.off_graph_notice(),
            "This article has no previous sibling."
        );
        assert_eq!(
            ThreadMotion::NextSibling.off_graph_notice(),
            "This article has no next sibling."
        );
    }

    #[test]
    fn parent_motion_moves_to_a_loaded_parent_or_names_the_missing_one() {
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [
                    loaded_article(5, "parent", Some("expired")),
                    loaded_article(6, "child", Some("parent")),
                ],
            )
            .unwrap();
        state.mark_article_read("comp.test", 5).unwrap();
        let mut reader = reader_for(&state);
        reader.start_reading(&state).unwrap();
        assert_eq!(reader.selected_article_number(), Some(6));
        assert_eq!(
            reader.move_target(&state, ThreadMotion::Parent),
            MoveTarget::Loaded(5)
        );
        assert!(reader.select_article(5));
        assert_eq!(reader.selected_article_number(), Some(5));
        assert_eq!(
            reader.move_target(&state, ThreadMotion::Parent),
            MoveTarget::Missing(MessageId::parse(b"<expired>").unwrap())
        );
    }

    #[test]
    fn adopting_a_fetched_parent_joins_it_to_the_thread_without_marking_read() {
        // 7 replies to <parent>, which replies to <root>; neither is loaded.
        let mut state = SessionState::default();
        let mut child = loaded_article(7, "child", None);
        child.metadata.references = vec![
            MessageId::parse(b"<root>").unwrap(),
            MessageId::parse(b"<parent>").unwrap(),
        ];
        state.load_articles("comp.test", [child]).unwrap();
        let mut reader = reader_for(&state);
        reader.start_reading(&state).unwrap();
        let MoveTarget::Missing(id) = reader.move_target(&state, ThreadMotion::Parent) else {
            panic!("parent should be a placeholder");
        };
        assert_eq!(id, MessageId::parse(b"<parent>").unwrap());

        let mut parent = loaded_article(3, "parent", Some("root"));
        parent.metadata.subject = Some(b"Re: topic".to_vec());
        reader
            .adopt_fetched_article(&mut state, 3, parent.metadata)
            .unwrap();
        assert_eq!(reader.selected_article_number(), Some(3));
        let navigation = reader.article_navigation.as_ref().unwrap();
        assert_eq!(navigation.article_numbers(), [3, 7]);
        assert_eq!(
            reader.move_target(&state, ThreadMotion::Parent),
            MoveTarget::Missing(MessageId::parse(b"<root>").unwrap())
        );
        assert!(!state.is_read("comp.test", 3));
        assert!(state.newsrc.entry("comp.test").is_none());
        assert_eq!(reader.reading_threads, [3]);
    }
}
