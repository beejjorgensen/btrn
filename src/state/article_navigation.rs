use super::{SessionState, StateError};

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
        let root = group_state
            .threads
            .nodes
            .get(&root_article_number)
            .filter(|node| node.parent.is_none())
            .ok_or(StateError::ThreadNotLoaded)?;
        let article_numbers =
            super::thread_list::walk_thread(&group_state.threads, root.article_number);
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

    pub fn article_numbers(&self) -> &[u64] {
        &self.article_numbers
    }

    pub fn selected_article_number(&self) -> Option<u64> {
        self.selected_index
            .and_then(|index| self.article_numbers.get(index).copied())
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
