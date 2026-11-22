use crate::nntp::active::{ActiveGroupList, PostingStatus};
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
}

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
        Self { rows }
    }

    #[cfg(test)]
    pub fn from_groups(groups: &[crate::nntp::active::ActiveGroup], newsrc: &NewsrcState) -> Self {
        Self::from_active(
            &ActiveGroupList {
                groups: groups.to_vec(),
                invalid_lines: Vec::new(),
            },
            newsrc,
        )
    }

    pub fn toggle(&mut self, index: usize) -> Option<bool> {
        let row = self.rows.get_mut(index)?;
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
    use crate::nntp::active::ActiveGroup;
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
    fn marks_multiple_groups_without_changing_newsrc_subscription_state() {
        let mut selector = GroupSelector::from_groups(
            &[
                group("comp.test", 1, 1),
                group("misc.test", 2, 2),
                group("z.test", 3, 3),
            ],
            &newsrc(),
        );
        assert_eq!(selector.toggle(0), Some(true));
        assert_eq!(selector.toggle(2), Some(true));
        assert_eq!(selector.marked_groups(), ["comp.test", "z.test"]);
        assert_eq!(selector.toggle(2), Some(false));
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
