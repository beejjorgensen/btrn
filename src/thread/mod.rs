use crate::model::article::{ArticleMetadata, MessageId};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadArticle {
    pub article_number: u64,
    pub metadata: ArticleMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadNode {
    pub article_number: u64,
    pub message_id: Option<MessageId>,
    pub parent: Option<u64>,
    pub unresolved_parent: Option<MessageId>,
    pub children: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadIssue {
    MissingMessageId {
        article_number: u64,
    },
    DuplicateMessageId {
        message_id: MessageId,
        article_numbers: Vec<u64>,
    },
    UnresolvedParent {
        article_number: u64,
        message_id: MessageId,
    },
    CycleBroken {
        article_number: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThreadTree {
    pub nodes: BTreeMap<u64, ThreadNode>,
    pub roots: Vec<u64>,
    pub issues: Vec<ThreadIssue>,
}

pub fn build_threads(articles: impl IntoIterator<Item = ThreadArticle>) -> ThreadTree {
    let mut articles = articles.into_iter().collect::<Vec<_>>();
    articles.sort_by_key(|article| article.article_number);

    let mut ids: BTreeMap<MessageId, Vec<u64>> = BTreeMap::new();
    let mut nodes = BTreeMap::new();
    let mut issues = Vec::new();

    for article in &articles {
        let message_id = article.metadata.message_id.clone();
        if let Some(id) = &message_id {
            ids.entry(id.clone())
                .or_default()
                .push(article.article_number);
        } else {
            issues.push(ThreadIssue::MissingMessageId {
                article_number: article.article_number,
            });
        }
        nodes.insert(
            article.article_number,
            ThreadNode {
                article_number: article.article_number,
                message_id,
                parent: None,
                unresolved_parent: None,
                children: Vec::new(),
            },
        );
    }

    for (message_id, article_numbers) in &ids {
        if article_numbers.len() > 1 {
            issues.push(ThreadIssue::DuplicateMessageId {
                message_id: message_id.clone(),
                article_numbers: article_numbers.clone(),
            });
        }
    }

    for article in &articles {
        let parent_id = article
            .metadata
            .references
            .last()
            .or(article.metadata.in_reply_to.as_ref());
        let Some(parent_id) = parent_id else {
            continue;
        };
        let article_number = article.article_number;
        let node = nodes
            .get_mut(&article_number)
            .expect("article node was inserted");
        match ids.get(parent_id).and_then(|numbers| numbers.first()) {
            Some(parent_number) => {
                node.parent = Some(*parent_number);
            }
            None => {
                node.unresolved_parent = Some(parent_id.clone());
                issues.push(ThreadIssue::UnresolvedParent {
                    article_number,
                    message_id: parent_id.clone(),
                });
            }
        }
    }

    break_parent_cycles(&mut nodes, &mut issues);

    let edges = nodes
        .iter()
        .filter_map(|(number, node)| node.parent.map(|parent| (parent, *number)))
        .collect::<Vec<_>>();
    for (parent, child) in edges {
        if let Some(parent_node) = nodes.get_mut(&parent) {
            parent_node.children.push(child);
        }
    }
    let roots = nodes
        .iter()
        .filter_map(|(number, node)| node.parent.is_none().then_some(*number))
        .collect();

    ThreadTree {
        nodes,
        roots,
        issues,
    }
}

fn break_parent_cycles(nodes: &mut BTreeMap<u64, ThreadNode>, issues: &mut Vec<ThreadIssue>) {
    let mut finished = HashSet::new();
    let starts = nodes.keys().copied().collect::<Vec<_>>();

    for start in starts {
        if finished.contains(&start) {
            continue;
        }
        let mut path = Vec::new();
        let mut positions = HashMap::new();
        let mut current = Some(start);
        while let Some(number) = current {
            if finished.contains(&number) {
                break;
            }
            if let Some(cycle_start) = positions.get(&number).copied() {
                let cycle = &path[cycle_start..];
                let break_at = *cycle.iter().max().expect("cycle is non-empty");
                if let Some(node) = nodes.get_mut(&break_at) {
                    node.parent = None;
                }
                issues.push(ThreadIssue::CycleBroken {
                    article_number: break_at,
                });
                break;
            }
            positions.insert(number, path.len());
            path.push(number);
            current = nodes.get(&number).and_then(|node| node.parent);
        }
        finished.extend(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn article(
        number: u64,
        message_id: Option<&[u8]>,
        references: &[&[u8]],
        in_reply_to: Option<&[u8]>,
        subject: Option<&[u8]>,
    ) -> ThreadArticle {
        ThreadArticle {
            article_number: number,
            metadata: ArticleMetadata {
                message_id: message_id.map(|id| MessageId::parse(id).unwrap()),
                references: references
                    .iter()
                    .map(|id| MessageId::parse(id).unwrap())
                    .collect(),
                in_reply_to: in_reply_to.map(|id| MessageId::parse(id).unwrap()),
                from: None,
                subject: subject.map(<[u8]>::to_vec),
                date: None,
                issues: Vec::new(),
            },
        }
    }

    #[test]
    fn links_by_message_id_and_ignores_matching_subjects() {
        let tree = build_threads([
            article(1, Some(b"<root>"), &[], None, Some(b"same subject")),
            article(2, Some(b"<reply>"), &[b"<root>"], None, Some(b"other")),
            article(3, Some(b"<stranger>"), &[], None, Some(b"same subject")),
        ]);
        assert_eq!(tree.roots, [1, 3]);
        assert_eq!(tree.nodes[&1].children, [2]);
        assert_eq!(tree.nodes[&2].parent, Some(1));
        assert_eq!(tree.nodes[&3].parent, None);
        assert!(tree.issues.is_empty());
    }

    #[test]
    fn keeps_missing_parents_unresolved_until_metadata_is_loaded() {
        let tree = build_threads([article(
            12,
            Some(b"<child>"),
            &[b"<not-loaded>"],
            None,
            None,
        )]);
        assert_eq!(tree.roots, [12]);
        assert_eq!(tree.nodes[&12].parent, None);
        assert_eq!(
            tree.nodes[&12].unresolved_parent,
            Some(MessageId::parse(b"<not-loaded>").unwrap())
        );
        assert_eq!(
            tree.issues,
            [ThreadIssue::UnresolvedParent {
                article_number: 12,
                message_id: MessageId::parse(b"<not-loaded>").unwrap(),
            }]
        );
    }

    #[test]
    fn resolves_duplicates_to_lowest_article_number_deterministically() {
        let tree = build_threads([
            article(20, Some(b"<duplicate>"), &[], None, None),
            article(5, Some(b"<duplicate>"), &[], None, None),
            article(30, Some(b"<child>"), &[b"<duplicate>"], None, None),
        ]);
        assert_eq!(tree.nodes[&30].parent, Some(5));
        assert!(tree.issues.contains(&ThreadIssue::DuplicateMessageId {
            message_id: MessageId::parse(b"<duplicate>").unwrap(),
            article_numbers: [5, 20].to_vec(),
        }));
    }

    #[test]
    fn uses_in_reply_to_when_references_are_absent_and_breaks_cycles() {
        let tree = build_threads([
            article(1, Some(b"<one>"), &[], Some(b"<two>"), None),
            article(2, Some(b"<two>"), &[], Some(b"<one>"), None),
        ]);
        assert_eq!(tree.roots, [2]);
        assert_eq!(tree.nodes[&2].children, [1]);
        assert_eq!(
            tree.issues,
            [ThreadIssue::CycleBroken { article_number: 2 }]
        );
    }

    #[test]
    fn keeps_articles_without_message_ids_as_independent_roots() {
        let tree = build_threads([article(9, None, &[], None, None)]);
        assert_eq!(tree.roots, [9]);
        assert_eq!(tree.nodes[&9].parent, None);
        assert_eq!(tree.nodes[&9].unresolved_parent, None);
        assert_eq!(
            tree.issues,
            [ThreadIssue::MissingMessageId { article_number: 9 }]
        );
    }

    #[test]
    fn references_take_precedence_over_in_reply_to() {
        let tree = build_threads([
            article(1, Some(b"<reference-parent>"), &[], None, None),
            article(2, Some(b"<reply-parent>"), &[], None, None),
            article(
                3,
                Some(b"<child>"),
                &[b"<reference-parent>"],
                Some(b"<reply-parent>"),
                None,
            ),
        ]);
        assert_eq!(tree.nodes[&3].parent, Some(1));
    }
}
