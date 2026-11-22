use super::{GroupState, SessionState, StateError};
use crate::decoder::{decode_header_value, DecodeIssue};
use crate::model::article::ArticleMetadata;
use chrono::DateTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRow {
    pub group_name: String,
    /// The thread's first loaded article in walk order. It is the root
    /// article unless the root is a placeholder for an unloaded article.
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
}

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
        Ok(Self {
            group_name: group.to_owned(),
            rows,
        })
    }
}

fn build_rows(group_state: &GroupState, group: &str, state: &SessionState) -> Vec<ThreadRow> {
    group_state
        .threads
        .roots
        .iter()
        .filter_map(|root| {
            let article_numbers = group_state.threads.walk_articles(root);
            let first_article = *article_numbers.first()?;
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
            let newest_article_number = article_numbers
                .iter()
                .copied()
                .max()
                .unwrap_or(first_article);
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
            Some(ThreadRow {
                group_name: group.to_owned(),
                root_article_number: first_article,
                article_numbers,
                subject,
                unread_count,
                newest_date,
                newest_article_number,
                has_unresolved_parent,
                decode_issues,
            })
        })
        .collect()
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
    }

    #[test]
    fn orphans_sharing_an_unloaded_root_form_one_row() {
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [
                    LoadedArticle {
                        article_number: 8,
                        metadata: metadata(
                            "a",
                            &["root", "gone"],
                            "Re: topic",
                            "Wed, 02 Jan 2019 10:00:00 +0000",
                        ),
                    },
                    LoadedArticle {
                        article_number: 9,
                        metadata: metadata(
                            "b",
                            &["root"],
                            "Re: topic",
                            "Wed, 02 Jan 2019 11:00:00 +0000",
                        ),
                    },
                ],
            )
            .unwrap();
        let list = ThreadList::from_session(&state, "comp.test").unwrap();
        assert_eq!(list.rows.len(), 1);
        assert_eq!(list.rows[0].root_article_number, 8);
        assert_eq!(list.rows[0].article_numbers, [8, 9]);
        assert_eq!(list.rows[0].unread_count, 2);
        assert!(list.rows[0].has_unresolved_parent);
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
