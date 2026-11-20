use crate::nntp::active::{ActiveGroup, ActiveGroupList, PostingStatus};
use crate::persistence::newsrc::NewsrcState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionStatus {
    Subscribed,
    Unsubscribed,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRow {
    pub name: String,
    pub first_article: u64,
    pub last_article: u64,
    pub posting_status: PostingStatus,
    pub subscription: SubscriptionStatus,
    pub marked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupSelector {
    pub rows: Vec<GroupRow>,
    selected_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupSelectionError {
    IndexOutOfBounds,
}

impl std::fmt::Display for GroupSelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IndexOutOfBounds => write!(formatter, "group selection index is out of bounds"),
        }
    }
}

impl std::error::Error for GroupSelectionError {}

impl GroupSelector {
    pub fn from_active(active: &ActiveGroupList, newsrc: &NewsrcState) -> Self {
        let mut groups = active.groups.iter().collect::<Vec<_>>();
        groups.sort_by(|left, right| left.name.cmp(&right.name));
        let rows = groups
            .into_iter()
            .map(|group| GroupRow {
                name: group.name.clone(),
                first_article: group.first_article,
                last_article: group.last_article,
                posting_status: group.posting_status.clone(),
                subscription: newsrc.entry(&group.name).map_or(
                    SubscriptionStatus::Untracked,
                    |entry| {
                        if entry.subscribed {
                            SubscriptionStatus::Subscribed
                        } else {
                            SubscriptionStatus::Unsubscribed
                        }
                    },
                ),
                marked: false,
            })
            .collect::<Vec<_>>();
        let selected_index = (!rows.is_empty()).then_some(0);
        Self {
            rows,
            selected_index,
        }
    }

    pub fn from_groups(groups: &[ActiveGroup], newsrc: &NewsrcState) -> Self {
        Self::from_active(
            &ActiveGroupList {
                groups: groups.to_vec(),
                invalid_lines: Vec::new(),
            },
            newsrc,
        )
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    pub fn selected(&self) -> Option<&GroupRow> {
        self.selected_index.and_then(|index| self.rows.get(index))
    }

    pub fn select(&mut self, index: usize) -> Result<(), GroupSelectionError> {
        if index >= self.rows.len() {
            return Err(GroupSelectionError::IndexOutOfBounds);
        }
        self.selected_index = Some(index);
        Ok(())
    }

    pub fn select_next(&mut self) {
        let Some(last_index) = self.rows.len().checked_sub(1) else {
            return;
        };
        self.selected_index = Some(
            self.selected_index
                .map_or(0, |index| index.saturating_add(1).min(last_index)),
        );
    }

    pub fn select_previous(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        self.selected_index = Some(
            self.selected_index
                .map_or(0, |index| index.saturating_sub(1)),
        );
    }

    pub fn toggle_selected(&mut self) -> Option<bool> {
        let row = self.rows.get_mut(self.selected_index?)?;
        row.marked = !row.marked;
        Some(row.marked)
    }

    pub fn marked_groups(&self) -> Vec<&str> {
        self.rows
            .iter()
            .filter(|row| row.marked)
            .map(|row| row.name.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::newsrc::NewsrcEntry;
    use std::collections::BTreeMap;

    fn group(name: &str, first_article: u64, last_article: u64) -> ActiveGroup {
        ActiveGroup {
            name: name.to_owned(),
            first_article,
            last_article,
            posting_status: PostingStatus::Allowed,
        }
    }

    fn newsrc() -> NewsrcState {
        NewsrcState {
            entries: BTreeMap::from([
                (
                    "comp.test".to_owned(),
                    NewsrcEntry {
                        subscribed: true,
                        read_ranges: Vec::new(),
                    },
                ),
                (
                    "misc.test".to_owned(),
                    NewsrcEntry {
                        subscribed: false,
                        read_ranges: Vec::new(),
                    },
                ),
            ]),
        }
    }

    #[test]
    fn sorts_groups_and_distinguishes_subscription_status() {
        let active = ActiveGroupList {
            groups: vec![
                group("z.test", 1, 4),
                group("misc.test", 0, 0),
                group("comp.test", 5, 9),
            ],
            invalid_lines: Vec::new(),
        };
        let selector = GroupSelector::from_active(&active, &newsrc());

        assert_eq!(
            selector
                .rows
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            ["comp.test", "misc.test", "z.test"]
        );
        assert_eq!(
            selector.rows[0].subscription,
            SubscriptionStatus::Subscribed
        );
        assert_eq!(
            selector.rows[1].subscription,
            SubscriptionStatus::Unsubscribed
        );
        assert_eq!(selector.rows[2].subscription, SubscriptionStatus::Untracked);
        assert_eq!(selector.rows[1].first_article, 0);
        assert_eq!(selector.rows[1].last_article, 0);
    }

    #[test]
    fn cursor_movement_clamps_and_explicit_selection_checks_bounds() {
        let mut selector = GroupSelector::from_groups(
            &[group("a.test", 1, 1), group("b.test", 2, 2)],
            &NewsrcState::default(),
        );
        assert_eq!(selector.selected_index(), Some(0));
        assert_eq!(
            selector.selected().map(|row| row.name.as_str()),
            Some("a.test")
        );
        selector.select_previous();
        assert_eq!(selector.selected_index(), Some(0));
        selector.select_next();
        assert_eq!(selector.selected_index(), Some(1));
        selector.select_next();
        assert_eq!(selector.selected_index(), Some(1));
        assert_eq!(
            selector.select(2),
            Err(GroupSelectionError::IndexOutOfBounds)
        );
        assert!(selector.select(0).is_ok());

        let mut empty = GroupSelector::default();
        empty.select_next();
        empty.select_previous();
        assert_eq!(empty.selected_index(), None);
        assert_eq!(empty.selected(), None);
        assert_eq!(empty.toggle_selected(), None);
    }

    #[test]
    fn marks_multiple_groups_without_changing_newsrc_subscription_state() {
        let mut selector = GroupSelector::from_groups(
            &[
                group("comp.test", 1, 1),
                group("misc.test", 2, 2),
                group("z.test", 3, 3),
            ],
            &newsrc(),
        );
        assert_eq!(selector.toggle_selected(), Some(true));
        selector.select(2).unwrap();
        assert_eq!(selector.toggle_selected(), Some(true));
        assert_eq!(selector.marked_groups(), ["comp.test", "z.test"]);
        assert_eq!(selector.toggle_selected(), Some(false));
        assert_eq!(selector.marked_groups(), ["comp.test"]);
        assert_eq!(
            selector.rows[0].subscription,
            SubscriptionStatus::Subscribed
        );
        assert_eq!(
            selector.rows[1].subscription,
            SubscriptionStatus::Unsubscribed
        );
        assert_eq!(selector.rows[2].subscription, SubscriptionStatus::Untracked);
    }

    #[test]
    fn preserves_unknown_posting_status() {
        let mut unknown = group("a.test", 1, 2);
        unknown.posting_status = PostingStatus::Unknown(b"private".to_vec());
        let selector = GroupSelector::from_groups(&[unknown.clone()], &NewsrcState::default());
        assert_eq!(selector.rows[0].posting_status, unknown.posting_status);
    }
}
