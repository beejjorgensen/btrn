pub mod thread_list;

use crate::model::article::{ArticleMetadata, ArticleMetadataIssue, MessageId};
use crate::nntp::overview::{OverviewArticle, OverviewRows};
use crate::persistence::newsrc::{normalize_ranges, ArticleRange, NewsrcEntry, NewsrcState};
use crate::thread::{build_threads, ThreadArticle, ThreadTree};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedArticle {
    pub article_number: u64,
    pub metadata: ArticleMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupState {
    pub articles: BTreeMap<u64, ArticleMetadata>,
    pub threads: ThreadTree,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionState {
    pub newsrc: NewsrcState,
    pub groups: BTreeMap<String, GroupState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateError {
    InvalidGroupName,
    InvalidArticleNumber,
    GroupNotLoaded,
}

impl std::fmt::Display for StateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGroupName => write!(formatter, "invalid newsgroup name"),
            Self::InvalidArticleNumber => write!(formatter, "invalid article number"),
            Self::GroupNotLoaded => write!(formatter, "newsgroup metadata has not been loaded"),
        }
    }
}

impl std::error::Error for StateError {}

impl SessionState {
    pub fn load_overview(&mut self, group: &str, rows: OverviewRows) -> Result<(), StateError> {
        validate_group_name(group)?;
        let articles = rows
            .articles
            .into_iter()
            .map(overview_metadata)
            .collect::<Vec<_>>();
        self.load_articles(group, articles)
    }

    pub fn load_articles(
        &mut self,
        group: &str,
        articles: impl IntoIterator<Item = LoadedArticle>,
    ) -> Result<(), StateError> {
        validate_group_name(group)?;
        let incoming = articles.into_iter().collect::<Vec<_>>();
        if incoming.iter().any(|article| article.article_number == 0) {
            return Err(StateError::InvalidArticleNumber);
        }

        let group_state = self.groups.entry(group.to_owned()).or_default();
        for article in incoming {
            group_state
                .articles
                .insert(article.article_number, article.metadata);
        }
        group_state.threads = build_threads(group_state.articles.iter().map(
            |(article_number, metadata)| ThreadArticle {
                article_number: *article_number,
                metadata: metadata.clone(),
            },
        ));
        Ok(())
    }

    pub fn is_read(&self, group: &str, article_number: u64) -> bool {
        self.newsrc
            .entry(group)
            .is_some_and(|entry| contains_article(&entry.read_ranges, article_number))
    }

    pub fn mark_article_read(
        &mut self,
        group: &str,
        article_number: u64,
    ) -> Result<(), StateError> {
        validate_group_name(group)?;
        if article_number == 0 {
            return Err(StateError::InvalidArticleNumber);
        }
        let entry = self
            .newsrc
            .entries
            .entry(group.to_owned())
            .or_insert_with(|| NewsrcEntry {
                subscribed: false,
                read_ranges: Vec::new(),
            });
        entry.read_ranges.push(ArticleRange {
            first: article_number,
            last: article_number,
        });
        entry.read_ranges = normalize_ranges(std::mem::take(&mut entry.read_ranges));
        Ok(())
    }
}

fn validate_group_name(group: &str) -> Result<(), StateError> {
    if group.is_empty()
        || !group.is_ascii()
        || group
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(StateError::InvalidGroupName);
    }
    Ok(())
}

fn contains_article(ranges: &[ArticleRange], article_number: u64) -> bool {
    ranges
        .iter()
        .any(|range| range.first <= article_number && article_number <= range.last)
}

fn overview_metadata(article: OverviewArticle) -> LoadedArticle {
    let message_id = article
        .message_id
        .as_deref()
        .and_then(MessageId::from_normalized);
    let mut issues = Vec::new();
    if let Some(raw) = &article.message_id {
        if message_id.is_none() {
            issues.push(ArticleMetadataIssue::InvalidMessageId {
                field: "Message-ID",
                value: raw.clone(),
            });
        }
    }
    issues.extend(article.invalid_references.into_iter().map(|value| {
        ArticleMetadataIssue::InvalidReference {
            field: "References",
            value,
        }
    }));

    LoadedArticle {
        article_number: article.article_number,
        metadata: ArticleMetadata {
            message_id,
            references: article
                .references
                .iter()
                .filter_map(|value| MessageId::from_normalized(value))
                .collect(),
            in_reply_to: None,
            from: article.from,
            subject: article.subject,
            date: article.date,
            issues,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nntp::overview::{InvalidOverviewRow, OverviewRows};

    fn metadata(id: &str, references: &[&str]) -> ArticleMetadata {
        ArticleMetadata {
            message_id: MessageId::parse(format!("<{id}>").as_bytes()),
            references: references
                .iter()
                .map(|value| MessageId::parse(format!("<{value}>").as_bytes()).unwrap())
                .collect(),
            in_reply_to: None,
            from: None,
            subject: None,
            date: None,
            issues: Vec::new(),
        }
    }

    #[test]
    fn loading_articles_builds_threads_without_marking_any_read() {
        let mut state = SessionState::default();
        state
            .load_articles(
                "comp.test",
                [
                    LoadedArticle {
                        article_number: 1,
                        metadata: metadata("root", &[]),
                    },
                    LoadedArticle {
                        article_number: 2,
                        metadata: metadata("child", &["root"]),
                    },
                ],
            )
            .unwrap();
        assert_eq!(state.groups["comp.test"].threads.nodes[&2].parent, Some(1));
        assert!(!state.is_read("comp.test", 1));
        assert!(!state.is_read("comp.test", 2));
        assert!(state.newsrc.entry("comp.test").is_none());
    }

    #[test]
    fn marking_read_updates_only_that_groups_normalized_ranges() {
        let mut state = SessionState::default();
        state.newsrc.entries.insert(
            "comp.test".to_owned(),
            NewsrcEntry {
                subscribed: true,
                read_ranges: Vec::new(),
            },
        );
        state
            .mark_article_read("comp.test", 8)
            .and_then(|()| state.mark_article_read("comp.test", 7))
            .and_then(|()| state.mark_article_read("misc.test", 7))
            .unwrap();

        assert!(state.is_read("comp.test", 7));
        assert!(state.is_read("comp.test", 8));
        assert!(!state.is_read("comp.test", 9));
        assert!(state.is_read("misc.test", 7));
        assert!(!state.is_read("misc.test", 8));
        assert_eq!(
            state.newsrc.entry("comp.test").unwrap().read_ranges,
            [ArticleRange { first: 7, last: 8 }]
        );
        assert_eq!(
            state.newsrc.entry("misc.test").unwrap().read_ranges,
            [ArticleRange { first: 7, last: 7 }]
        );
        assert!(state.newsrc.entry("comp.test").unwrap().subscribed);
    }

    #[test]
    fn loading_overview_creates_typed_metadata_and_threads() {
        let mut state = SessionState::default();
        state
            .load_overview(
                "comp.test",
                OverviewRows {
                    articles: vec![
                        OverviewArticle {
                            article_number: 1,
                            subject: Some(b"root".to_vec()),
                            from: None,
                            date: None,
                            message_id: Some(b"root@example".to_vec()),
                            references: Vec::new(),
                            invalid_references: Vec::new(),
                            bytes: None,
                            lines: None,
                            unknown_fields: Vec::new(),
                            invalid_fields: Vec::new(),
                        },
                        OverviewArticle {
                            article_number: 2,
                            subject: Some(b"reply".to_vec()),
                            from: None,
                            date: None,
                            message_id: Some(b"reply@example".to_vec()),
                            references: vec![b"root@example".to_vec()],
                            invalid_references: vec![b"invalid".to_vec()],
                            bytes: None,
                            lines: None,
                            unknown_fields: Vec::new(),
                            invalid_fields: Vec::new(),
                        },
                    ],
                    invalid_rows: Vec::<InvalidOverviewRow>::new(),
                    out_of_range_rows: Vec::new(),
                },
            )
            .unwrap();

        let group = &state.groups["comp.test"];
        assert_eq!(
            group.articles[&2].subject.as_deref(),
            Some(b"reply".as_slice())
        );
        assert_eq!(group.threads.nodes[&2].parent, Some(1));
        assert!(matches!(
            group.articles[&2].issues.as_slice(),
            [ArticleMetadataIssue::InvalidReference { .. }]
        ));
        assert!(!state.is_read("comp.test", 2));
    }

    #[test]
    fn rejects_invalid_group_names_and_article_zero_without_mutation() {
        let mut state = SessionState::default();
        assert_eq!(
            state.mark_article_read("bad group", 1),
            Err(StateError::InvalidGroupName)
        );
        assert_eq!(
            state.mark_article_read("comp.test", 0),
            Err(StateError::InvalidArticleNumber)
        );
        assert!(state.newsrc.entries.is_empty());
        assert!(state.groups.is_empty());
    }
}
