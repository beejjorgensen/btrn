use super::{SessionState, StateError};
use crate::thread::ThreadRef;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArticleNavigation {
    group_name: String,
    article_numbers: Vec<u64>,
    selected_index: Option<usize>,
}

impl ArticleNavigation {
    pub fn from_thread(
        state: &SessionState,
        group: &str,
        root_article_number: u64,
    ) -> Result<Self, StateError> {
        super::validate_group_name(group)?;
        let group_state = state.groups.get(group).ok_or(StateError::GroupNotLoaded)?;
        let tree = &group_state.threads;
        if tree.thread_first_article(root_article_number) != Some(root_article_number) {
            return Err(StateError::ThreadNotLoaded);
        }
        let top = tree.top(&ThreadRef::Article(root_article_number));
        let article_numbers = tree.walk_articles(&top);
        let selected_index = article_numbers
            .iter()
            .position(|number| !state.is_read(group, *number))
            .or_else(|| (!article_numbers.is_empty()).then_some(0));
        Ok(Self {
            group_name: group.to_owned(),
            article_numbers,
            selected_index,
        })
    }

    pub fn group_name(&self) -> &str {
        &self.group_name
    }

    /// The thread's first loaded article; navigation always starts its walk
    /// at the thread root.
    pub fn root_article_number(&self) -> Option<u64> {
        self.article_numbers.first().copied()
    }

    pub fn article_numbers(&self) -> &[u64] {
        &self.article_numbers
    }

    pub fn selected_article_number(&self) -> Option<u64> {
        self.selected_index
            .and_then(|index| self.article_numbers.get(index).copied())
    }

    /// Selects `article_number` if it is in this thread.
    pub fn select_article(&mut self, article_number: u64) -> bool {
        match self
            .article_numbers
            .iter()
            .position(|number| *number == article_number)
        {
            Some(index) => {
                self.selected_index = Some(index);
                true
            }
            None => false,
        }
    }

    pub fn next_article(&mut self) -> Option<u64> {
        let next = self.selected_index?.checked_add(1)?;
        if next >= self.article_numbers.len() {
            return None;
        }
        self.selected_index = Some(next);
        self.selected_article_number()
    }

    pub fn previous_article(&mut self) -> Option<u64> {
        let previous = self.selected_index?.checked_sub(1)?;
        self.selected_index = Some(previous);
        self.selected_article_number()
    }

    pub fn next_unread_article(&mut self, state: &SessionState) -> Option<u64> {
        let start = self.selected_index?.checked_add(1)?;
        let index = (start..self.article_numbers.len())
            .find(|index| !state.is_read(&self.group_name, self.article_numbers[*index]))?;
        self.selected_index = Some(index);
        self.selected_article_number()
    }

    pub fn previous_unread_article(&mut self, state: &SessionState) -> Option<u64> {
        let index = (0..self.selected_index?)
            .rev()
            .find(|index| !state.is_read(&self.group_name, self.article_numbers[*index]))?;
        self.selected_index = Some(index);
        self.selected_article_number()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadRootError {
    State(StateError),
    OutsideLoadedWindow,
    NotRoot,
    NoArticle,
}

impl std::fmt::Display for ThreadRootError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(error) => write!(formatter, "{error}"),
            Self::OutsideLoadedWindow => {
                write!(
                    formatter,
                    "thread root is outside the loaded overview window"
                )
            }
            Self::NotRoot => write!(formatter, "article number does not identify a thread root"),
            Self::NoArticle => write!(formatter, "thread has no loaded articles"),
        }
    }
}

impl std::error::Error for ThreadRootError {}

pub fn first_unread_article_for_thread_root(
    state: &SessionState,
    group: &str,
    root_article_number: u64,
) -> Result<u64, ThreadRootError> {
    let group_state = state
        .groups
        .get(group)
        .ok_or(ThreadRootError::State(StateError::GroupNotLoaded))?;
    match group_state
        .threads
        .thread_first_article(root_article_number)
    {
        None => return Err(ThreadRootError::OutsideLoadedWindow),
        Some(first) if first != root_article_number => return Err(ThreadRootError::NotRoot),
        Some(_) => {}
    }
    ArticleNavigation::from_thread(state, group, root_article_number)
        .map_err(ThreadRootError::State)?
        .selected_article_number()
        .ok_or(ThreadRootError::NoArticle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::article::{ArticleMetadata, MessageId};
    use crate::state::LoadedArticle;

    fn article(number: u64, id: &str, parent: Option<&str>) -> LoadedArticle {
        LoadedArticle {
            article_number: number,
            metadata: ArticleMetadata {
                message_id: MessageId::parse(format!("<{id}>").as_bytes()),
                references: parent
                    .map(|parent| vec![MessageId::parse(format!("<{parent}>").as_bytes()).unwrap()])
                    .unwrap_or_default(),
                in_reply_to: None,
                from: None,
                subject: None,
                date: None,
                issues: Vec::new(),
            },
        }
    }

    fn state() -> SessionState {
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [
                    article(10, "root", None),
                    article(20, "child-a", Some("root")),
                    article(30, "grandchild", Some("child-a")),
                    article(40, "child-b", Some("root")),
                ],
            )
            .unwrap();
        state
    }

    #[test]
    fn starts_at_first_unread_then_moves_in_deterministic_thread_order() {
        let mut state = state();
        state.mark_article_read("comp.test", 10).unwrap();
        state.mark_article_read("comp.test", 20).unwrap();

        let mut navigation = ArticleNavigation::from_thread(&state, "comp.test", 10).unwrap();
        assert_eq!(navigation.group_name(), "comp.test");
        assert_eq!(navigation.article_numbers(), [10, 20, 30, 40]);
        assert_eq!(navigation.selected_article_number(), Some(30));
        assert_eq!(navigation.next_article(), Some(40));
        assert_eq!(navigation.next_article(), None);
        assert_eq!(navigation.selected_article_number(), Some(40));
        assert_eq!(navigation.previous_article(), Some(30));
    }

    #[test]
    fn starts_at_first_article_when_thread_is_all_read() {
        let mut state = state();
        for number in [10, 20, 30, 40] {
            state.mark_article_read("comp.test", number).unwrap();
        }

        let navigation = ArticleNavigation::from_thread(&state, "comp.test", 10).unwrap();
        assert_eq!(navigation.selected_article_number(), Some(10));
    }

    #[test]
    fn unread_navigation_skips_read_articles_without_wrapping_or_marking_read() {
        let mut state = state();
        for number in [10, 20, 40] {
            state.mark_article_read("comp.test", number).unwrap();
        }

        let mut navigation = ArticleNavigation::from_thread(&state, "comp.test", 10).unwrap();
        assert_eq!(navigation.selected_article_number(), Some(30));
        assert_eq!(navigation.previous_unread_article(&state), None);
        assert_eq!(navigation.selected_article_number(), Some(30));
        assert_eq!(navigation.next_article(), Some(40));
        assert_eq!(navigation.previous_unread_article(&state), Some(30));
        assert_eq!(navigation.previous_unread_article(&state), None);
        assert_eq!(navigation.selected_article_number(), Some(30));
        assert!(!state.is_read("comp.test", 30));
    }

    #[test]
    fn chooses_first_unread_article_and_reports_root_outside_loaded_window() {
        let mut state = state();
        state.mark_article_read("comp.test", 10).unwrap();
        assert_eq!(
            first_unread_article_for_thread_root(&state, "comp.test", 10),
            Ok(20)
        );
        assert_eq!(
            first_unread_article_for_thread_root(&state, "comp.test", 999),
            Err(ThreadRootError::OutsideLoadedWindow)
        );
        assert_eq!(
            first_unread_article_for_thread_root(&state, "comp.test", 20),
            Err(ThreadRootError::NotRoot)
        );
        assert!(!state.is_read("comp.test", 20));
    }

    #[test]
    fn a_thread_under_a_placeholder_is_identified_by_its_first_article() {
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [
                    article(5, "a", Some("expired")),
                    article(6, "b", Some("expired")),
                ],
            )
            .unwrap();
        let navigation = ArticleNavigation::from_thread(&state, "comp.test", 5).unwrap();
        assert_eq!(navigation.article_numbers(), [5, 6]);
        assert_eq!(navigation.root_article_number(), Some(5));
        assert_eq!(
            ArticleNavigation::from_thread(&state, "comp.test", 6),
            Err(StateError::ThreadNotLoaded)
        );
        assert_eq!(
            first_unread_article_for_thread_root(&state, "comp.test", 6),
            Err(ThreadRootError::NotRoot)
        );
    }

    #[test]
    fn rejects_missing_or_invalid_thread_context() {
        assert_eq!(
            ArticleNavigation::from_thread(&SessionState::default(), "comp.test", 1),
            Err(StateError::GroupNotLoaded)
        );
        assert_eq!(
            ArticleNavigation::from_thread(&state(), "comp.test", 20),
            Err(StateError::ThreadNotLoaded)
        );
        assert_eq!(
            ArticleNavigation::from_thread(&state(), "bad group", 10),
            Err(StateError::InvalidGroupName)
        );
    }
}
