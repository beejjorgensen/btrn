use super::{GroupState, SessionState, StateError};
use crate::decoder::{decode_header_value, DecodeIssue};
use crate::model::article::ArticleMetadata;
use crate::thread::ThreadTree;
use chrono::DateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRow {
    pub group_name: String,
    pub root_article_number: u64,
    pub article_numbers: Vec<u64>,
    pub subject: String,
    pub unread_count: usize,
    pub newest_date: Option<i64>,
    pub newest_article_number: u64,
    pub has_unresolved_parent: bool,
    pub decode_issues: Vec<DecodeIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThreadList {
    pub group_name: String,
    pub rows: Vec<ThreadRow>,
    selected_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadSelectionError {
    IndexOutOfBounds,
}

impl std::fmt::Display for ThreadSelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IndexOutOfBounds => write!(formatter, "thread selection index is out of bounds"),
        }
    }
}

impl std::error::Error for ThreadSelectionError {}

impl ThreadList {
    pub fn from_session(state: &SessionState, group: &str) -> Result<Self, StateError> {
        super::validate_group_name(group)?;
        let group_state = state.groups.get(group).ok_or(StateError::GroupNotLoaded)?;
        let mut rows = build_rows(group_state, group, state);
        rows.sort_by(|left, right| {
            right
                .newest_date
                .cmp(&left.newest_date)
                .then_with(|| right.newest_article_number.cmp(&left.newest_article_number))
                .then_with(|| left.root_article_number.cmp(&right.root_article_number))
        });
        let selected_index = rows
            .iter()
            .position(|row| row.unread_count > 0)
            .or_else(|| (!rows.is_empty()).then_some(0));
        Ok(Self {
            group_name: group.to_owned(),
            rows,
            selected_index,
        })
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected_index
    }

    pub fn selected(&self) -> Option<&ThreadRow> {
        self.selected_index.and_then(|index| self.rows.get(index))
    }

    pub fn select(&mut self, index: usize) -> Result<(), ThreadSelectionError> {
        if index >= self.rows.len() {
            return Err(ThreadSelectionError::IndexOutOfBounds);
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
}

fn build_rows(group_state: &GroupState, group: &str, state: &SessionState) -> Vec<ThreadRow> {
    group_state
        .threads
        .roots
        .iter()
        .map(|root| {
            let article_numbers = walk_thread(&group_state.threads, *root);
            let subject_article = article_numbers
                .iter()
                .find(|number| {
                    group_state
                        .articles
                        .get(number)
                        .is_some_and(|metadata| metadata.subject.is_some())
                })
                .copied();
            let (subject, decode_issues) = subject_article
                .and_then(|number| group_state.articles.get(&number))
                .and_then(|metadata| metadata.subject.as_deref())
                .map(|subject| decode_header_value(b"Subject", subject))
                .unwrap_or_else(|| ("(no subject)".to_owned(), Vec::new()));
            let newest_date = article_numbers
                .iter()
                .filter_map(|number| group_state.articles.get(number))
                .filter_map(parse_date)
                .max();
            let newest_article_number = article_numbers.iter().copied().max().unwrap_or(*root);
            let unread_count = article_numbers
                .iter()
                .filter(|number| !state.is_read(group, **number))
                .count();
            let has_unresolved_parent = article_numbers.iter().any(|number| {
                group_state
                    .threads
                    .nodes
                    .get(number)
                    .is_some_and(|node| node.unresolved_parent.is_some())
            });
            ThreadRow {
                group_name: group.to_owned(),
                root_article_number: *root,
                article_numbers,
                subject,
                unread_count,
                newest_date,
                newest_article_number,
                has_unresolved_parent,
                decode_issues,
            }
        })
        .collect()
}

fn walk_thread(tree: &ThreadTree, root: u64) -> Vec<u64> {
    let mut articles = Vec::new();
    let mut pending = vec![root];
    while let Some(number) = pending.pop() {
        articles.push(number);
        if let Some(node) = tree.nodes.get(&number) {
            pending.extend(node.children.iter().rev().copied());
        }
    }
    articles
}

fn parse_date(metadata: &ArticleMetadata) -> Option<i64> {
    let date = String::from_utf8_lossy(metadata.date.as_deref()?);
    DateTime::parse_from_rfc2822(date.trim())
        .ok()
        .map(|date| date.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::article::{ArticleMetadata, MessageId};
    use crate::state::LoadedArticle;

    fn metadata(id: &str, references: &[&str], subject: &str, date: &str) -> ArticleMetadata {
        ArticleMetadata {
            message_id: MessageId::parse(format!("<{id}>").as_bytes()),
            references: references
                .iter()
                .map(|reference| MessageId::parse(format!("<{reference}>").as_bytes()).unwrap())
                .collect(),
            in_reply_to: None,
            from: None,
            subject: Some(subject.as_bytes().to_vec()),
            date: Some(date.as_bytes().to_vec()),
            issues: Vec::new(),
        }
    }

    #[test]
    fn builds_date_ordered_rows_with_decoded_subject_and_unread_counts() {
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [
                    LoadedArticle {
                        article_number: 1,
                        metadata: metadata(
                            "old-root",
                            &[],
                            "=?UTF-8?Q?caf=C3=A9?=",
                            "Tue, 01 Jan 2019 10:00:00 +0000",
                        ),
                    },
                    LoadedArticle {
                        article_number: 2,
                        metadata: metadata(
                            "old-reply",
                            &["old-root"],
                            "ignored reply subject",
                            "Tue, 01 Jan 2019 11:00:00 +0000",
                        ),
                    },
                    LoadedArticle {
                        article_number: 3,
                        metadata: metadata(
                            "new-root",
                            &[],
                            "new topic",
                            "Wed, 02 Jan 2019 10:00:00 +0000",
                        ),
                    },
                ],
            )
            .unwrap();
        state.mark_article_read("comp.test", 3).unwrap();

        let list = ThreadList::from_session(&state, "comp.test").unwrap();
        assert_eq!(
            list.rows
                .iter()
                .map(|row| row.root_article_number)
                .collect::<Vec<_>>(),
            [3, 1]
        );
        assert_eq!(list.rows[1].subject, "caf\u{e9}");
        assert_eq!(list.rows[1].article_numbers, [1, 2]);
        assert_eq!(list.rows[1].unread_count, 2);
        assert_eq!(list.rows[0].unread_count, 0);
        assert_eq!(list.selected_index(), Some(1));
        assert_eq!(list.selected().map(|row| row.root_article_number), Some(1));
    }

    #[test]
    fn selection_is_explicit_clamped_and_safe_for_empty_lists() {
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [
                    LoadedArticle {
                        article_number: 1,
                        metadata: metadata("one", &[], "one", "invalid"),
                    },
                    LoadedArticle {
                        article_number: 2,
                        metadata: metadata("two", &[], "two", "invalid"),
                    },
                ],
            )
            .unwrap();
        let mut list = ThreadList::from_session(&state, "comp.test").unwrap();
        assert_eq!(list.selected_index(), Some(0));
        list.select_previous();
        assert_eq!(list.selected_index(), Some(0));
        list.select_next();
        assert_eq!(list.selected_index(), Some(1));
        list.select_next();
        assert_eq!(list.selected_index(), Some(1));
        list.select(0).unwrap();
        assert_eq!(list.select(2), Err(ThreadSelectionError::IndexOutOfBounds));
        assert_eq!(list.selected_index(), Some(0));

        state.load_articles("misc.test", []).unwrap();
        let mut empty = ThreadList::from_session(&state, "misc.test").unwrap();
        empty.select_next();
        empty.select_previous();
        assert_eq!(empty.selected_index(), None);
        assert_eq!(empty.selected(), None);
    }

    #[test]
    fn preserves_unresolved_parent_diagnostic_in_row() {
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [LoadedArticle {
                    article_number: 7,
                    metadata: metadata("orphan", &["missing"], "orphan", "invalid"),
                }],
            )
            .unwrap();
        let list = ThreadList::from_session(&state, "comp.test").unwrap();
        assert!(list.rows[0].has_unresolved_parent);
    }

    #[test]
    fn reports_when_group_metadata_is_not_loaded() {
        assert_eq!(
            ThreadList::from_session(&SessionState::default(), "comp.test"),
            Err(StateError::GroupNotLoaded)
        );
        assert_eq!(
            ThreadList::from_session(&SessionState::default(), "bad group"),
            Err(StateError::InvalidGroupName)
        );
    }
}
