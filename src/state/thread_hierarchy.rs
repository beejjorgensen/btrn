//! Character-grid layout of one thread's reply hierarchy for the article
//! view's upper-right window. This is derived display data built from the
//! loaded thread tree and read state; it performs no terminal I/O.

use super::{SessionState, StateError};
use crate::decoder::decode_header_value;
use crate::thread::{ThreadRef, ThreadTree};
use std::collections::HashMap;

/// Columns of the article view's thread window.
pub const HIERARCHY_WIDTH: usize = 25;
/// Rows of the article view's thread window.
pub const HIERARCHY_HEIGHT: usize = 6;
/// Row of the window the current article is centered on when possible: the
/// upper of the two middle rows.
const CENTER_ROW: usize = (HIERARCHY_HEIGHT - 1) / 2;
const UNRESOLVED_MARKER: &str = "(?)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HierarchyChar {
    pub character: char,
    pub inverse: bool,
}

impl HierarchyChar {
    fn plain(character: char) -> Self {
        Self {
            character,
            inverse: false,
        }
    }
}

pub type HierarchyRow = Vec<HierarchyChar>;

/// The current article's cell position in the grid, `start..end` columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CellSpan {
    row: usize,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchyGrid {
    rows: Vec<HierarchyRow>,
    current: Option<CellSpan>,
    current_subject_index: Option<usize>,
}

/// Per-article facts the layout needs, separated from `SessionState` so the
/// layout itself is a plain function of the tree.
pub struct HierarchyInput<'a> {
    pub tree: &'a ThreadTree,
    pub root: ThreadRef,
    pub subject_index: &'a dyn Fn(u64) -> usize,
    pub is_read: &'a dyn Fn(u64) -> bool,
    pub current: u64,
    pub previous: Option<u64>,
}

/// Builds the hierarchy for the thread containing article `root` in a
/// loaded group, drawn from the thread's top node, which may be a
/// placeholder. The current article is drawn as read because presenting it
/// marks it read.
pub fn build_hierarchy(
    state: &SessionState,
    group: &str,
    root: u64,
    current: u64,
    previous: Option<u64>,
) -> Result<HierarchyGrid, StateError> {
    let group_state = state.groups.get(group).ok_or(StateError::GroupNotLoaded)?;
    let tree = &group_state.threads;
    if !tree.nodes.contains_key(&root) {
        return Err(StateError::ThreadNotLoaded);
    }
    let root = tree.top(&ThreadRef::Article(root));
    let subjects = subject_indexes(tree, &root, |number| {
        group_state
            .articles
            .get(&number)
            .and_then(|metadata| metadata.subject.as_deref())
            .map(|subject| decode_header_value(b"Subject", subject).0)
    });
    let subject_index = |number| subjects.get(&number).copied().unwrap_or(1);
    let is_read = |number| number == current || state.is_read(group, number);
    Ok(layout_hierarchy(&HierarchyInput {
        tree,
        root,
        subject_index: &subject_index,
        is_read: &is_read,
        current,
        previous,
    }))
}

/// Numbers distinct normalized subjects from 1 in thread order. An article
/// without a subject shares its loaded parent's number.
fn subject_indexes(
    tree: &ThreadTree,
    root: &ThreadRef,
    subject: impl Fn(u64) -> Option<String>,
) -> HashMap<u64, usize> {
    let mut numbers: Vec<String> = Vec::new();
    let mut indexes = HashMap::new();
    for number in tree.walk_articles(root) {
        let index = match subject(number) {
            Some(subject) => {
                let normalized = normalize_subject(&subject);
                match numbers.iter().position(|known| *known == normalized) {
                    Some(position) => position + 1,
                    None => {
                        numbers.push(normalized);
                        numbers.len()
                    }
                }
            }
            None => tree
                .nodes
                .get(&number)
                .and_then(|node| node.parent)
                .and_then(|parent| indexes.get(&parent).copied())
                .unwrap_or(1),
        };
        indexes.insert(number, index);
    }
    indexes
}

/// Strips any number of leading `Re:` prefixes and surrounding whitespace.
fn normalize_subject(subject: &str) -> String {
    let mut rest = subject.trim();
    while rest.len() >= 3 && rest[..3].eq_ignore_ascii_case("re:") {
        rest = rest[3..].trim_start();
    }
    rest.to_owned()
}

enum Placement {
    /// Continues on its parent's row.
    First { row: usize },
    /// Starts below everything drawn so far, joined to a parent on
    /// `parent_row` through column `connector`.
    Later {
        parent_row: usize,
        connector: usize,
        last: bool,
    },
}

/// Lays out the thread as rows of characters. A node's first child continues
/// on the same row; later children start on new rows below the previous
/// sibling's subtree, joined by `|`, `|-`, and `\-` connectors.
pub fn layout_hierarchy(input: &HierarchyInput<'_>) -> HierarchyGrid {
    let mut grid = Grid::default();
    let mut current = None;
    let mut current_subject_index = None;

    let mut pending = vec![(input.root.clone(), 0, Placement::First { row: 0 })];
    while let Some((node, col, placement)) = pending.pop() {
        let row = match placement {
            Placement::First { row } => row,
            Placement::Later {
                parent_row,
                connector,
                last,
            } => {
                let row = grid.rows.len();
                for between in parent_row + 1..row {
                    grid.put_if_blank(between, connector, '|');
                }
                grid.put(row, connector, if last { "\\-" } else { "|-" }, false);
                row
            }
        };

        let end = match &node {
            ThreadRef::Missing(_) => {
                grid.put(row, col, UNRESOLVED_MARKER, false);
                col + UNRESOLVED_MARKER.len()
            }
            ThreadRef::Article(number) => {
                let number = *number;
                let subject_index = (input.subject_index)(number);
                let (open, close) = if (input.is_read)(number) {
                    ('[', ']')
                } else {
                    ('(', ')')
                };
                let digits = subject_index.to_string();
                let is_current = number == input.current;
                let is_previous = input.previous == Some(number);
                grid.put(row, col, &open.to_string(), is_current);
                grid.put(row, col + 1, &digits, is_current || is_previous);
                let end = col + 1 + digits.len();
                grid.put(row, end, &close.to_string(), is_current);
                let end = end + 1;
                if is_current {
                    current = Some(CellSpan {
                        row,
                        start: col,
                        end,
                    });
                    current_subject_index = Some(subject_index);
                }
                end
            }
        };

        let children = input.tree.children(&node);
        if children.is_empty() {
            continue;
        }
        grid.put(
            row,
            end,
            if children.len() == 1 { "--" } else { "+-" },
            false,
        );
        let child_col = end + 2;
        for (index, child) in children.iter().enumerate().rev() {
            let placement = if index == 0 {
                Placement::First { row }
            } else {
                Placement::Later {
                    parent_row: row,
                    connector: end,
                    last: index + 1 == children.len(),
                }
            };
            pending.push((child.clone(), child_col, placement));
        }
    }

    HierarchyGrid {
        rows: grid.rows,
        current,
        current_subject_index,
    }
}

#[derive(Default)]
struct Grid {
    rows: Vec<HierarchyRow>,
}

impl Grid {
    fn cell(&mut self, row: usize, col: usize) -> &mut HierarchyChar {
        if self.rows.len() <= row {
            self.rows.resize_with(row + 1, Vec::new);
        }
        let line = &mut self.rows[row];
        if line.len() <= col {
            line.resize(col + 1, HierarchyChar::plain(' '));
        }
        &mut line[col]
    }

    fn put(&mut self, row: usize, col: usize, text: &str, inverse: bool) {
        for (offset, character) in text.chars().enumerate() {
            *self.cell(row, col + offset) = HierarchyChar { character, inverse };
        }
    }

    fn put_if_blank(&mut self, row: usize, col: usize, character: char) {
        let cell = self.cell(row, col);
        if cell.character == ' ' {
            *cell = HierarchyChar::plain(character);
        }
    }
}

impl HierarchyGrid {
    #[cfg(test)]
    pub fn rows(&self) -> &[HierarchyRow] {
        &self.rows
    }

    /// The subject number of the current article, if it is in the thread.
    pub fn current_subject_index(&self) -> Option<usize> {
        self.current_subject_index
    }

    fn width(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0)
    }

    /// The `width` x `height` view of the grid that centers the current
    /// article when possible. Each axis is clamped to the grid's extent, so
    /// near the edges of the graph the highlight moves off-center instead of
    /// the window showing empty space. A grid smaller than the window is
    /// anchored at its top-left. Rows are clipped to `width` characters and
    /// are not padded.
    pub fn window(&self, width: usize, height: usize) -> Vec<HierarchyRow> {
        let (center_col, center_row) = self
            .current
            .map(|span| (span.start + (span.end - span.start) / 2, span.row))
            .unwrap_or((0, 0));
        let origin_col = window_origin(center_col, width / 2, width, self.width());
        let origin_row = window_origin(
            center_row,
            CENTER_ROW.min(height.saturating_sub(1)),
            height,
            self.rows.len(),
        );
        self.rows
            .iter()
            .skip(origin_row)
            .take(height)
            .map(|row| row.iter().skip(origin_col).take(width).copied().collect())
            .collect()
    }
}

fn window_origin(center: usize, offset: usize, window: usize, extent: usize) -> usize {
    center
        .saturating_sub(offset)
        .min(extent.saturating_sub(window))
}

/// The characters of a hierarchy row, for tests and diagnostics.
#[cfg(test)]
pub fn row_text(row: &[HierarchyChar]) -> String {
    row.iter().map(|cell| cell.character).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::article::{ArticleMetadata, MessageId};
    use crate::state::LoadedArticle;
    use std::collections::HashSet;

    fn article(number: u64, id: &str, parent: Option<&str>, subject: &str) -> LoadedArticle {
        LoadedArticle {
            article_number: number,
            metadata: ArticleMetadata {
                message_id: MessageId::parse(format!("<{id}>").as_bytes()),
                references: parent
                    .map(|parent| vec![MessageId::parse(format!("<{parent}>").as_bytes()).unwrap()])
                    .unwrap_or_default(),
                in_reply_to: None,
                from: None,
                subject: (!subject.is_empty()).then(|| subject.as_bytes().to_vec()),
                date: None,
                issues: Vec::new(),
            },
        }
    }

    fn session(articles: Vec<LoadedArticle>) -> SessionState {
        let mut state = SessionState::default();
        state.load_articles("comp.test", articles).unwrap();
        state
    }

    fn texts(rows: &[HierarchyRow]) -> Vec<String> {
        rows.iter().map(|row| row_text(row)).collect()
    }

    /// Characters drawn inverse, with everything else as spaces.
    fn inverse(rows: &[HierarchyRow]) -> Vec<String> {
        rows.iter()
            .map(|row| {
                row.iter()
                    .map(|cell| if cell.inverse { cell.character } else { ' ' })
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    /// The branched example from the screens specification, minus the long
    /// tail: root 1 with children 2, 5, 6.
    fn branched() -> SessionState {
        session(vec![
            article(1, "a", None, "topic"),
            article(2, "b", Some("a"), "Re: topic"),
            article(3, "c", Some("b"), "Re: topic"),
            article(4, "d", Some("b"), "Re: topic"),
            article(5, "e", Some("a"), "Re: topic"),
            article(6, "f", Some("a"), "Re: topic"),
            article(7, "g", Some("f"), "Re: topic"),
        ])
    }

    #[test]
    fn single_article_is_one_cell() {
        let state = session(vec![article(1, "a", None, "topic")]);
        let grid = build_hierarchy(&state, "comp.test", 1, 1, None).unwrap();
        assert_eq!(texts(grid.rows()), ["[1]"]);
        assert_eq!(inverse(grid.rows()), ["[1]"]);
        assert_eq!(grid.current_subject_index(), Some(1));
    }

    #[test]
    fn linear_chain_continues_on_one_row() {
        let state = session(vec![
            article(1, "a", None, "topic"),
            article(2, "b", Some("a"), "Re: topic"),
            article(3, "c", Some("b"), "Re: Re: topic"),
        ]);
        let grid = build_hierarchy(&state, "comp.test", 1, 2, None).unwrap();
        assert_eq!(texts(grid.rows()), ["(1)--[1]--(1)"]);
    }

    #[test]
    fn branches_use_plus_bar_and_backslash_connectors() {
        let grid = build_hierarchy(&branched(), "comp.test", 1, 1, None).unwrap();
        assert_eq!(
            texts(grid.rows()),
            [
                "[1]+-(1)+-(1)",
                "   |    \\-(1)",
                "   |-(1)",
                "   \\-(1)--(1)",
            ]
        );
    }

    #[test]
    fn read_articles_use_brackets_and_unread_use_parentheses() {
        let mut state = branched();
        state.mark_article_read("comp.test", 3).unwrap();
        state.mark_article_read("comp.test", 7).unwrap();
        let grid = build_hierarchy(&state, "comp.test", 1, 5, None).unwrap();
        assert_eq!(
            texts(grid.rows()),
            [
                "(1)+-(1)+-[1]",
                "   |    \\-(1)",
                "   |-[1]",
                "   \\-(1)--[1]",
            ]
        );
    }

    #[test]
    fn current_is_fully_inverse_and_previous_has_only_its_number_inverse() {
        let grid = build_hierarchy(&branched(), "comp.test", 1, 4, Some(2)).unwrap();
        assert_eq!(inverse(grid.rows()), ["      1", "          [1]", "", ""]);
    }

    #[test]
    fn distinct_subjects_are_numbered_in_thread_order() {
        let state = session(vec![
            article(1, "a", None, "topic"),
            article(2, "b", Some("a"), "other"),
            article(3, "c", Some("b"), "RE: other"),
            article(4, "d", Some("a"), "Re:topic"),
            article(5, "e", Some("d"), ""),
            article(6, "f", Some("a"), "third"),
        ]);
        let grid = build_hierarchy(&state, "comp.test", 1, 6, None).unwrap();
        assert_eq!(
            texts(grid.rows()),
            ["(1)+-(2)--(2)", "   |-(1)--(1)", "   \\-[3]"]
        );
        assert_eq!(grid.current_subject_index(), Some(3));
    }

    #[test]
    fn unresolved_parent_is_marked_beside_the_root() {
        let state = session(vec![
            article(1, "a", Some("expired"), "Re: topic"),
            article(2, "b", Some("a"), "Re: topic"),
        ]);
        let grid = build_hierarchy(&state, "comp.test", 1, 2, None).unwrap();
        assert_eq!(texts(grid.rows()), ["(?)--(1)--[1]"]);
    }

    #[test]
    fn placeholders_join_orphaned_siblings_under_one_marker() {
        // Two replies to an expired article, one of them with a reply.
        let mut state = session(vec![
            article(4, "x", Some("expired"), "Re: topic"),
            article(6, "y", Some("expired"), "Re: topic"),
            article(7, "z", Some("x"), "Re: topic"),
        ]);
        state.mark_article_read("comp.test", 4).unwrap();
        let grid = build_hierarchy(&state, "comp.test", 4, 6, None).unwrap();
        assert_eq!(texts(grid.rows()), ["(?)+-[1]--(1)", "   \\-[1]"]);
        assert_eq!(inverse(grid.rows()), ["", "     [1]"]);
    }

    #[test]
    fn unknown_group_or_root_is_an_error() {
        let state = branched();
        assert_eq!(
            build_hierarchy(&state, "comp.other", 1, 1, None),
            Err(StateError::GroupNotLoaded)
        );
        assert_eq!(
            build_hierarchy(&state, "comp.test", 99, 1, None),
            Err(StateError::ThreadNotLoaded)
        );
    }

    fn chain(length: u64) -> SessionState {
        session(
            (1..=length)
                .map(|number| {
                    let parent = (number > 1).then(|| format!("m{}", number - 1));
                    article(number, &format!("m{number}"), parent.as_deref(), "topic")
                })
                .collect(),
        )
    }

    /// A root with `count` direct replies, one per row.
    fn fan(count: u64) -> SessionState {
        let mut articles = vec![article(1, "root", None, "topic")];
        articles.extend(
            (2..=count + 1)
                .map(|number| article(number, &format!("r{number}"), Some("root"), "topic")),
        );
        session(articles)
    }

    fn window_for(state: &SessionState, current: u64) -> Vec<HierarchyRow> {
        build_hierarchy(state, "comp.test", 1, current, None)
            .unwrap()
            .window(HIERARCHY_WIDTH, HIERARCHY_HEIGHT)
    }

    /// Column range of the inverse cell in a window.
    fn highlight(rows: &[HierarchyRow]) -> (usize, usize, usize) {
        rows.iter()
            .enumerate()
            .find_map(|(row, cells)| {
                let start = cells.iter().position(|cell| cell.inverse)?;
                let end = cells.iter().rposition(|cell| cell.inverse)? + 1;
                Some((row, start, end))
            })
            .expect("window shows the current article")
    }

    #[test]
    fn window_centers_the_current_article_in_a_long_chain() {
        // Each article occupies 5 columns: "(1)--".
        let window = window_for(&chain(20), 10);
        assert_eq!(texts(&window), ["-(1)--(1)--[1]--(1)--(1)-"]);
        let (_, start, end) = highlight(&window);
        assert_eq!((start, end), (11, 14));
    }

    #[test]
    fn window_clamps_at_the_horizontal_edges() {
        let window = window_for(&chain(20), 1);
        assert_eq!(texts(&window), ["[1]--(1)--(1)--(1)--(1)--"]);
        assert_eq!(highlight(&window), (0, 0, 3));

        let window = window_for(&chain(20), 20);
        assert_eq!(texts(&window), ["--(1)--(1)--(1)--(1)--[1]"]);
        let (_, start, end) = highlight(&window);
        assert_eq!((start, end), (22, 25));
    }

    #[test]
    fn window_centers_vertically_and_clamps_at_top_and_bottom() {
        let state = fan(12);
        // Reply article n is on grid row n - 2.
        let window = window_for(&state, 8);
        assert_eq!(window.len(), HIERARCHY_HEIGHT);
        assert_eq!(highlight(&window).0, 2);

        let window = window_for(&state, 1);
        assert_eq!(highlight(&window).0, 0);
        assert!(row_text(&window[0]).starts_with("[1]+-"));

        let window = window_for(&state, 13);
        assert_eq!(highlight(&window).0, HIERARCHY_HEIGHT - 1);
        assert_eq!(row_text(&window[HIERARCHY_HEIGHT - 1]), "   \\-[1]");
    }

    #[test]
    fn small_graph_is_anchored_top_left() {
        let window = window_for(&branched(), 7);
        assert_eq!(
            texts(&window),
            [
                "(1)+-(1)+-(1)",
                "   |    \\-(1)",
                "   |-(1)",
                "   \\-(1)--[1]",
            ]
        );
    }

    #[test]
    fn window_never_exceeds_its_dimensions() {
        let mut articles = vec![article(1, "root", None, "topic")];
        // Twelve branches, each a twelve-article chain.
        let mut number = 2;
        for branch in 0..12 {
            let mut parent = "root".to_owned();
            for depth in 0..12 {
                let id = format!("b{branch}d{depth}");
                articles.push(article(number, &id, Some(&parent), "topic"));
                parent = id;
                number += 1;
            }
        }
        let state = session(articles);
        let all = (1..number).collect::<HashSet<_>>();
        for current in all {
            let window = window_for(&state, current);
            assert!(window.len() <= HIERARCHY_HEIGHT);
            assert!(window.iter().all(|row| row.len() <= HIERARCHY_WIDTH));
            highlight(&window);
        }
    }

    #[test]
    fn subject_normalization_strips_repeated_reply_prefixes() {
        assert_eq!(normalize_subject("  Re: RE:re: topic "), "topic");
        assert_eq!(normalize_subject("Reply"), "Reply");
        assert_eq!(normalize_subject("Re:"), "");
    }
}
