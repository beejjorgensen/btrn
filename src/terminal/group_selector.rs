use crate::nntp::active::ActiveGroupList;
use crate::persistence::newsrc::NewsrcState;
use crate::state::group_selector::{GroupSelector, SubscriptionStatus};
use crate::terminal::selector;
use crate::terminal::session::TerminalSession;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use crossterm::queue;
use crossterm::terminal::{self, Clear, ClearType};
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GroupSelectorOperation {
    Quit,
    ToggleGroup(char),
    NextPage,
    PreviousPage,
    StartSelectedGroups,
    Ignore,
}

fn operation_for_key(key: KeyEvent) -> GroupSelectorOperation {
    if key.kind != KeyEventKind::Press {
        return GroupSelectorOperation::Ignore;
    }
    if crate::terminal::is_interrupt(&key) {
        return GroupSelectorOperation::Quit;
    }
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => GroupSelectorOperation::Quit,
        KeyCode::Char('>') => GroupSelectorOperation::NextPage,
        KeyCode::Char('<') => GroupSelectorOperation::PreviousPage,
        KeyCode::Char(' ') => GroupSelectorOperation::StartSelectedGroups,
        KeyCode::Char(letter) if letter.is_ascii_alphanumeric() && letter != 'q' => {
            GroupSelectorOperation::ToggleGroup(letter.to_ascii_lowercase())
        }
        _ => GroupSelectorOperation::Ignore,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct GroupPage {
    entries: Vec<usize>,
    end_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GroupSelectorScreen {
    groups: GroupSelector,
    page_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SpaceAction {
    AdvancePage,
    StartGroups(Vec<String>),
}

impl GroupSelectorScreen {
    fn new(mut groups: GroupSelector, newsrc: &NewsrcState) -> Self {
        groups.rows.retain(|row| {
            row.subscription == SubscriptionStatus::Subscribed
                && group_unread_count(row.first_article, row.last_article, newsrc, &row.name) > 0
        });
        Self {
            groups,
            page_index: 0,
        }
    }

    fn toggle_visible_group(&mut self, letter: char, page: &GroupPage) -> Option<(usize, bool)> {
        let (_, index) = page.entries.iter().enumerate().find(|(visible_index, _)| {
            selector::selection_letter(*visible_index) == Some(letter)
        })?;
        Some((*index, self.groups.toggle(*index)?))
    }

    fn next_page(&mut self, page_count: usize) {
        if self.page_index + 1 < page_count {
            self.page_index += 1;
        }
    }

    fn previous_page(&mut self) {
        self.page_index = self.page_index.saturating_sub(1);
    }

    fn normalize_page(&mut self, page_count: usize) {
        self.page_index = self.page_index.min(page_count.saturating_sub(1));
    }

    fn groups_to_read(&self) -> Vec<String> {
        let marked = self.groups.marked_groups();
        if marked.is_empty() {
            self.groups
                .rows
                .iter()
                .map(|row| row.name.clone())
                .collect()
        } else {
            marked.into_iter().map(str::to_owned).collect()
        }
    }

    fn start_or_advance(&mut self, page_count: usize) -> SpaceAction {
        if !self.groups.marked_groups().is_empty() || self.page_index + 1 >= page_count {
            SpaceAction::StartGroups(self.groups_to_read())
        } else {
            self.next_page(page_count);
            SpaceAction::AdvancePage
        }
    }
}

fn group_pages(groups: &GroupSelector, height: u16) -> Vec<GroupPage> {
    const MAX_PAGE_ITEMS: usize = 35;
    let capacity = usize::from(height).saturating_sub(4).min(MAX_PAGE_ITEMS);
    if capacity == 0 {
        return vec![GroupPage {
            entries: Vec::new(),
            end_index: groups.rows.len(),
        }];
    }
    groups
        .rows
        .chunks(capacity)
        .enumerate()
        .map(|(page_index, _entries)| {
            let end_index = ((page_index + 1) * capacity).min(groups.rows.len());
            GroupPage {
                entries: (page_index * capacity..end_index).collect(),
                end_index,
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .chain(groups.rows.is_empty().then_some(GroupPage::default()))
        .collect()
}

fn group_unread_count(
    first_article: u64,
    last_article: u64,
    newsrc: &NewsrcState,
    group: &str,
) -> u64 {
    if first_article == 0 || last_article < first_article {
        return 0;
    }
    let total = last_article.saturating_sub(first_article).saturating_add(1);
    let ranges = newsrc
        .entry(group)
        .into_iter()
        .flat_map(|entry| &entry.read_ranges)
        .filter_map(|range| {
            let first = range.first.max(first_article);
            let last = range.last.min(last_article);
            (first <= last).then_some(crate::persistence::newsrc::ArticleRange { first, last })
        })
        .collect();
    let read = crate::persistence::newsrc::normalize_ranges(ranges)
        .into_iter()
        .map(|range| range.last.saturating_sub(range.first).saturating_add(1))
        .fold(0u64, u64::saturating_add);
    total.saturating_sub(read.min(total))
}

fn group_row_y(page: &GroupPage, group_index: usize) -> Option<u16> {
    page.entries
        .iter()
        .position(|index| *index == group_index)
        .and_then(|visible_index| u16::try_from(visible_index + 2).ok())
}

pub fn run_interactive(
    active: &ActiveGroupList,
    newsrc: &NewsrcState,
) -> io::Result<Option<Vec<String>>> {
    let mut terminal = TerminalSession::enter()?;
    run_interactive_in_session(active, newsrc, &mut terminal)
}

pub(crate) fn run_interactive_in_session(
    active: &ActiveGroupList,
    newsrc: &NewsrcState,
    terminal: &mut TerminalSession,
) -> io::Result<Option<Vec<String>>> {
    let mut screen = GroupSelectorScreen::new(GroupSelector::from_active(active, newsrc), newsrc);
    let mut redraw = true;
    loop {
        if redraw {
            let (width, height) = terminal::size()?;
            let pages = group_pages(&screen.groups, height);
            screen.normalize_page(pages.len());
            render_group_list(&mut terminal.output, &screen, newsrc, &pages, width, height)?;
            redraw = false;
        }

        match event::read()? {
            Event::Resize(_, _) => redraw = true,
            Event::Key(key) => match operation_for_key(key) {
                GroupSelectorOperation::Quit => return Ok(None),
                GroupSelectorOperation::ToggleGroup(letter) => {
                    let (_, height) = terminal::size()?;
                    let pages = group_pages(&screen.groups, height);
                    screen.normalize_page(pages.len());
                    let updated = pages.get(screen.page_index).and_then(|page| {
                        screen
                            .toggle_visible_group(letter, page)
                            .map(|(index, marked)| (group_row_y(page, index), marked))
                    });
                    if let Some((Some(row), marked)) = updated {
                        selector::write_mark(&mut terminal.output, 1, row, marked)?;
                    } else if updated.is_some() {
                        redraw = true;
                    }
                }
                GroupSelectorOperation::NextPage => {
                    let (_, height) = terminal::size()?;
                    let old_page = screen.page_index;
                    screen.next_page(group_pages(&screen.groups, height).len());
                    redraw = old_page != screen.page_index;
                }
                GroupSelectorOperation::PreviousPage => {
                    let old_page = screen.page_index;
                    screen.previous_page();
                    redraw = old_page != screen.page_index;
                }
                GroupSelectorOperation::StartSelectedGroups => {
                    let (_, height) = terminal::size()?;
                    let action = screen.start_or_advance(group_pages(&screen.groups, height).len());
                    match action {
                        SpaceAction::AdvancePage => redraw = true,
                        SpaceAction::StartGroups(groups) => return Ok(Some(groups)),
                    }
                }
                GroupSelectorOperation::Ignore => {}
            },
            _ => {}
        }
    }
}

pub(crate) fn render_loading_threads(output: &mut impl Write, percent: usize) -> io::Result<()> {
    let (width, height) = terminal::size()?;
    queue!(
        output,
        Clear(ClearType::All),
        crossterm::cursor::MoveTo(0, height.saturating_sub(1))
    )?;
    selector::write_line(
        output,
        &format!("Loading threads {percent}%"),
        false,
        width,
        false,
    )?;
    output.flush()
}

fn render_group_list(
    output: &mut impl Write,
    screen: &GroupSelectorScreen,
    newsrc: &NewsrcState,
    pages: &[GroupPage],
    width: u16,
    height: u16,
) -> io::Result<()> {
    queue!(
        output,
        Clear(ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    )?;
    let page_index = screen.page_index.min(pages.len().saturating_sub(1));
    let page = pages.get(page_index).cloned().unwrap_or_default();
    let label = selector::page_label(
        page_index,
        pages.len(),
        page.end_index,
        screen.groups.rows.len(),
    );
    selector::write_line(output, "Newsgroups", false, width, true)?;
    selector::write_line(
        output,
        &format!("       {} groups", screen.groups.rows.len()),
        true,
        width,
        false,
    )?;
    if height > 1 {
        output.write_all(b"\r\n")?;
    }
    let mut list_lines = 0usize;
    if screen.groups.rows.is_empty() && height > 5 {
        selector::write_line(output, "No newsgroups", true, width, false)?;
        list_lines = 1;
    }
    for (visible_index, group_index) in page.entries.iter().enumerate() {
        let Some(row) = screen.groups.rows.get(*group_index) else {
            continue;
        };
        let letter = selector::selection_letter(visible_index).unwrap_or(' ');
        let unread = group_unread_count(row.first_article, row.last_article, newsrc, &row.name);
        selector::write_line(
            output,
            &format!(
                "{letter}{}  {unread:>3} {}",
                if row.marked { "+" } else { " " },
                row.name
            ),
            true,
            width,
            false,
        )?;
        list_lines += 1;
    }
    if height > 0 {
        let prompt_row = 3usize
            .saturating_add(list_lines)
            .min(usize::from(height.saturating_sub(1)));
        queue!(
            output,
            crossterm::cursor::MoveTo(0, u16::try_from(prompt_row).unwrap_or(u16::MAX))
        )?;
        selector::write_line(
            output,
            &format!(
                "-- Select a newsgroup (natural order) -- {label} --  < > / Space: page  q: quit"
            ),
            false,
            width,
            true,
        )?;
    }
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nntp::active::{ActiveGroup, PostingStatus};
    use crate::persistence::newsrc::{ArticleRange, NewsrcEntry};
    use crossterm::event::{KeyEvent, KeyModifiers};
    use std::collections::BTreeMap;

    fn group(name: &str, first_article: u64, last_article: u64) -> ActiveGroup {
        ActiveGroup {
            name: name.to_owned(),
            first_article,
            last_article,
            posting_status: PostingStatus::Allowed,
        }
    }

    fn groups(count: usize) -> GroupSelector {
        let active = ActiveGroupList {
            groups: (0..count)
                .map(|index| group(&format!("group.{index:02}"), 1, 10))
                .collect(),
            invalid_lines: Vec::new(),
        };
        GroupSelector::from_active(&active, &subscribed_newsrc(count))
    }

    fn subscribed_newsrc(count: usize) -> NewsrcState {
        NewsrcState {
            entries: (0..count)
                .map(|index| {
                    (
                        format!("group.{index:02}"),
                        NewsrcEntry {
                            subscribed: true,
                            read_ranges: Vec::new(),
                        },
                    )
                })
                .collect(),
        }
    }

    fn screen(count: usize) -> GroupSelectorScreen {
        let newsrc = subscribed_newsrc(count);
        GroupSelectorScreen::new(groups(count), &newsrc)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn group_selector_keys_are_screen_specific_and_q_is_reserved() {
        assert_eq!(
            operation_for_key(key(KeyCode::Char('a'))),
            GroupSelectorOperation::ToggleGroup('a')
        );
        assert_eq!(
            operation_for_key(key(KeyCode::Char('0'))),
            GroupSelectorOperation::ToggleGroup('0')
        );
        assert_eq!(
            operation_for_key(KeyEvent::new(
                KeyCode::Char('c'),
                crossterm::event::KeyModifiers::CONTROL
            )),
            GroupSelectorOperation::Quit
        );
        assert_eq!(
            operation_for_key(key(KeyCode::Char('q'))),
            GroupSelectorOperation::Quit
        );
        assert_eq!(
            operation_for_key(key(KeyCode::Char('<'))),
            GroupSelectorOperation::PreviousPage
        );
        assert_eq!(
            operation_for_key(key(KeyCode::Right)),
            GroupSelectorOperation::Ignore
        );
    }

    #[test]
    fn group_pagination_reserves_header_footer_and_caps_selection_letters() {
        let groups = groups(30);
        let pages = group_pages(&groups, 8);

        assert_eq!(pages.len(), 8);
        assert_eq!(pages[0].entries, [0, 1, 2, 3]);
        assert_eq!(pages[0].end_index, 4);
        assert_eq!(pages[7].entries, [28, 29]);
        assert_eq!(group_row_y(&pages[0], 0), Some(2));
        assert_eq!(selector::selection_letter(24), Some('z'));
    }

    #[test]
    fn group_pages_leave_a_blank_row_before_the_footer() {
        let groups = groups(10);
        let pages = group_pages(&groups, 8);

        assert_eq!(pages[0].entries.len(), 4);
        assert_eq!(group_row_y(&pages[0], 3), Some(5));
        assert_eq!(group_row_y(&pages[0], 3).unwrap() + 1, 6);
    }

    #[test]
    fn toggles_visible_groups_and_reads_marked_names_in_display_order() {
        let mut screen = screen(5);
        let pages = group_pages(&screen.groups, 8);

        assert_eq!(screen.toggle_visible_group('b', &pages[0]), Some((1, true)));
        assert_eq!(screen.toggle_visible_group('a', &pages[0]), Some((0, true)));
        assert_eq!(
            screen.groups_to_read(),
            ["group.00".to_owned(), "group.01".to_owned()]
        );
        assert_eq!(screen.toggle_visible_group('q', &pages[0]), None);
    }

    #[test]
    fn space_advances_until_last_page_then_starts_all_if_none_marked() {
        let mut screen = screen(6);
        let page_count = group_pages(&screen.groups, 6).len();

        for expected_page in 1..page_count {
            assert_eq!(
                screen.start_or_advance(page_count),
                SpaceAction::AdvancePage
            );
            assert_eq!(screen.page_index, expected_page);
        }
        assert_eq!(
            screen.start_or_advance(page_count),
            SpaceAction::StartGroups((0..6).map(|index| format!("group.{index:02}")).collect())
        );
    }

    #[test]
    fn space_starts_marked_groups_immediately_from_any_page() {
        let mut screen = screen(6);
        let page_count = group_pages(&screen.groups, 6).len();
        assert_eq!(
            screen.toggle_visible_group('a', &group_pages(&screen.groups, 6)[0]),
            Some((0, true))
        );

        assert_eq!(
            screen.start_or_advance(page_count),
            SpaceAction::StartGroups(vec!["group.00".to_owned()])
        );
        assert_eq!(screen.page_index, 0);
    }

    #[test]
    fn unread_counts_clip_read_ranges_to_current_server_range() {
        let newsrc = NewsrcState {
            entries: BTreeMap::from([(
                "group.test".to_owned(),
                NewsrcEntry {
                    subscribed: true,
                    read_ranges: vec![
                        ArticleRange { first: 1, last: 4 },
                        ArticleRange { first: 9, last: 20 },
                    ],
                },
            )]),
        };

        assert_eq!(group_unread_count(5, 10, &newsrc, "group.test"), 4);
        assert_eq!(group_unread_count(0, 0, &newsrc, "other.test"), 0);
    }

    #[test]
    fn group_screen_rendering_shares_inverse_headers_and_page_labels() {
        let newsrc = subscribed_newsrc(2);
        let screen = GroupSelectorScreen::new(groups(2), &newsrc);
        let pages = group_pages(&screen.groups, 8);
        let mut output = Vec::new();

        render_group_list(&mut output, &screen, &newsrc, &pages, 80, 8).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert!(rendered.contains("\u{1b}[7mNewsgroups\u{1b}[0m"));
        assert!(rendered.contains("2 groups\r\n\r\n"));
        assert!(rendered.contains("a    10 group.00"));
        assert!(rendered.contains("\u{1b}[6;1H"));
        assert!(rendered.contains("\u{1b}[7m-- Select a newsgroup"));
        assert!(rendered.contains("All"));
    }

    #[test]
    fn full_group_page_keeps_one_blank_row_before_bottom_prompt() {
        let newsrc = subscribed_newsrc(4);
        let screen = GroupSelectorScreen::new(groups(4), &newsrc);
        let pages = group_pages(&screen.groups, 8);
        let mut output = Vec::new();

        render_group_list(&mut output, &screen, &newsrc, &pages, 80, 8).unwrap();
        let rendered = String::from_utf8(output).unwrap();

        assert_eq!(pages[0].entries.len(), 4);
        assert!(rendered.contains("\u{1b}[8;1H"));
    }

    #[test]
    fn screen_shows_only_subscribed_groups_with_estimated_unread_articles() {
        let active = ActiveGroupList {
            groups: vec![
                group("sub.unread", 1, 10),
                group("sub.read", 1, 10),
                group("untracked.unread", 1, 10),
            ],
            invalid_lines: Vec::new(),
        };
        let newsrc = NewsrcState {
            entries: BTreeMap::from([
                (
                    "sub.unread".to_owned(),
                    NewsrcEntry {
                        subscribed: true,
                        read_ranges: vec![ArticleRange { first: 1, last: 9 }],
                    },
                ),
                (
                    "sub.read".to_owned(),
                    NewsrcEntry {
                        subscribed: true,
                        read_ranges: vec![ArticleRange { first: 1, last: 10 }],
                    },
                ),
            ]),
        };
        let screen =
            GroupSelectorScreen::new(GroupSelector::from_active(&active, &newsrc), &newsrc);

        assert_eq!(
            screen
                .groups
                .rows
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            ["sub.unread"]
        );
    }

    #[test]
    fn letters_and_digits_select_group_rows_after_q_is_skipped() {
        let mut screen = screen(35);
        let page = group_pages(&screen.groups, 50).remove(0);

        assert_eq!(screen.toggle_visible_group('0', &page), Some((25, true)));
        assert_eq!(screen.toggle_visible_group('9', &page), Some((34, true)));
        assert_eq!(screen.groups.rows[25].name, "group.25");
        assert_eq!(screen.groups.rows[34].name, "group.34");
    }
}
