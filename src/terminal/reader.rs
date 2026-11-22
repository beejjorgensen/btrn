use crate::decoder;
use crate::model::article::{ArticleMetadata, MessageId, RawArticle};
use crate::nntp::{ArticleError, NntpSession};
use crate::posting::PostingSettings;
use crate::state::article_navigation::ArticleNavigation;
use crate::state::thread_hierarchy::build_hierarchy;
use crate::state::thread_list::ThreadList;
use crate::state::{LoadedArticle, SessionState, StateError};
use crate::terminal::article_view::{
    present_article, prompt_line, render_article_page, ArticleLayout, ArticleViewContext,
};
use crate::terminal::selector;
use crate::terminal::session::TerminalSession;
use crate::thread::ThreadRef;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::queue;
use crossterm::terminal::{self, Clear, ClearType};
use std::collections::BTreeSet;
use std::fmt;
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
    /// `[`: go to the current article's parent, fetching it by message ID
    /// when it is not loaded.
    ParentArticle,
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
        (ReaderScreen::Article, KeyCode::Char('[')) => ReaderOperation::ParentArticle,
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

/// Where `[` leads from the current article.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParentTarget {
    /// The article is a thread root.
    None,
    Loaded(u64),
    /// The parent is a placeholder for an article that is not loaded.
    Missing(MessageId),
}

pub(crate) const PARENT_UNAVAILABLE: &str = "Parent article is not available.";

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

    pub(crate) fn parent_target(&self, state: &SessionState) -> ParentTarget {
        let Some(article_number) = self.selected_article_number() else {
            return ParentTarget::None;
        };
        let Some(group) = state.groups.get(&self.threads.group_name) else {
            return ParentTarget::None;
        };
        match group.threads.parent(&ThreadRef::Article(article_number)) {
            None => ParentTarget::None,
            Some(ThreadRef::Article(parent)) => ParentTarget::Loaded(parent),
            Some(ThreadRef::Missing(id)) => ParentTarget::Missing(id),
        }
    }

    pub(crate) fn select_article(&mut self, article_number: u64) -> bool {
        self.article_navigation
            .as_mut()
            .is_some_and(|navigation| navigation.select_article(article_number))
    }

    /// Adds a parent fetched by message ID to the group and selects it. The
    /// thread is rebuilt because the new article can replace placeholders
    /// and join the current thread to others. Loading it changes no read
    /// state.
    pub(crate) fn adopt_fetched_parent(
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

    pub fn next_article(&mut self) -> Option<u64> {
        self.article_navigation.as_mut()?.next_article()
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
        let hierarchy = self
            .article_navigation
            .as_ref()
            .and_then(ArticleNavigation::root_article_number)
            .and_then(|root| {
                build_hierarchy(
                    state,
                    &self.threads.group_name,
                    root,
                    article_number,
                    self.previous_article,
                )
                .ok()
            });
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
            ReaderOperation::ParentArticle => match reader_state.parent_target(state) {
                ParentTarget::None => {}
                ParentTarget::Loaded(parent) => redraw = reader_state.select_article(parent),
                ParentTarget::Missing(id) => {
                    let group = reader_state.threads.group_name.clone();
                    match fetch_parent_metadata(session, &group, &id)? {
                        Some((number, metadata)) => reader_state
                            .adopt_fetched_parent(state, number, metadata)
                            .map_err(ReaderError::State)?,
                        None => reader_state.notice = Some(PARENT_UNAVAILABLE),
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
fn fetch_parent_metadata(
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
    let decoded = crate::decoder::decode_header_value(b"From", value).0;
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

fn assigned_thread_letter(
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

fn page_thread_row_y(page: &ThreadPage, row_index: usize) -> Option<u16> {
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
    let page_position = selector::page_label(
        page_index,
        pages.len(),
        page.end_index,
        reader.threads.rows.len(),
    );
    let article_count = reader
        .threads
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
        format_args!("{}", reader.threads.group_name),
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
    if reader.threads.rows.is_empty() {
        write_terminal_line(output, format_args!("No threads"), true, width, false)?;
        list_lines = 1;
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
        render_thread_list(&mut output, &reader, &data, &pages, 80, 6).unwrap();
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
    fn incremental_mark_update_positions_cursor_and_writes_only_the_marker() {
        let data = session_with_threads(5);
        let reader = reader_for(&data);
        let pages = thread_pages(&reader.threads, &data, 10);
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
        let reader = reader_for(&data);
        let pages = thread_pages(&reader.threads, &data, 10);
        let mut output = Vec::new();

        render_thread_list(&mut output, &reader, &data, &pages, 80, 10).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("articles\r\n\r\n"));
        assert!(rendered.contains("Alice Example"));
        assert!(!rendered.contains("alice@example.com"));
        assert!(rendered.contains("\u{1b}[5;1H"));
    }

    #[test]
    fn full_thread_page_keeps_one_blank_row_before_bottom_prompt() {
        let data = session_with_threads(4);
        let reader = reader_for(&data);
        let pages = thread_pages(&reader.threads, &data, 8);
        let mut output = Vec::new();

        render_thread_list(&mut output, &reader, &data, &pages, 80, 8).unwrap();
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
    fn thread_selector_uses_digits_after_available_letters() {
        let data = session_with_threads(36);
        let reader = reader_for(&data);
        let pages = thread_pages(&reader.threads, &data, 50);

        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].entries.len(), 35);
        assert_eq!(
            assigned_thread_letter(&pages[0], &reader.threads, 34),
            Some('9')
        );
        assert_eq!(
            assigned_thread_letter(&pages[1], &reader.threads, 35),
            Some('a')
        );
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

        let mut reader = ReaderState::new(threads, 100);
        reader.thread_page = 0;
        let mut output = Vec::new();
        render_thread_list(&mut output, &reader, &data, &pages, 80, 7).unwrap();
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
        assert!(!rendered.contains("author-thread-1"));
        assert!(pages[0].entries.is_empty());
        assert_eq!(assigned_thread_letter(&pages[0], &reader.threads, 0), None);
    }

    #[test]
    fn terminal_line_is_sanitized_and_fitted_to_columns() {
        assert_eq!(selector::fit_terminal_line("ab\u{1b}cdef", 5), "ab…");
        assert_eq!(selector::fit_terminal_line("a\u{1b}b", 10), "a�b");
        assert_eq!(selector::fit_terminal_line("abcdef", 1), "…");
        assert_eq!(selector::fit_terminal_line("okay", 10), "okay");
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
    fn bracket_moves_to_a_loaded_parent_or_names_the_missing_one() {
        assert_eq!(
            operation_for_key(ReaderScreen::Article, key(KeyCode::Char('['))),
            ReaderOperation::ParentArticle
        );
        assert_eq!(
            operation_for_key(ReaderScreen::Threads, key(KeyCode::Char('['))),
            ReaderOperation::Ignore
        );

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
        assert_eq!(reader.parent_target(&state), ParentTarget::Loaded(5));
        assert!(reader.select_article(5));
        assert_eq!(reader.selected_article_number(), Some(5));
        assert_eq!(
            reader.parent_target(&state),
            ParentTarget::Missing(MessageId::parse(b"<expired>").unwrap())
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
        let ParentTarget::Missing(id) = reader.parent_target(&state) else {
            panic!("parent should be a placeholder");
        };
        assert_eq!(id, MessageId::parse(b"<parent>").unwrap());

        let mut parent = loaded_article(3, "parent", Some("root"));
        parent.metadata.subject = Some(b"Re: topic".to_vec());
        reader
            .adopt_fetched_parent(&mut state, 3, parent.metadata)
            .unwrap();
        assert_eq!(reader.selected_article_number(), Some(3));
        let navigation = reader.article_navigation.as_ref().unwrap();
        assert_eq!(navigation.article_numbers(), [3, 7]);
        assert_eq!(
            reader.parent_target(&state),
            ParentTarget::Missing(MessageId::parse(b"<root>").unwrap())
        );
        assert!(!state.is_read("comp.test", 3));
        assert!(state.newsrc.entry("comp.test").is_none());
        assert_eq!(reader.reading_threads, [3]);
    }
}
