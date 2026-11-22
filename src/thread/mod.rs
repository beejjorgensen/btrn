use crate::model::article::{ArticleMetadata, MessageId};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadArticle {
    pub article_number: u64,
    pub metadata: ArticleMetadata,
}

/// A node in the thread tree: a loaded article, or a placeholder for a
/// referenced article whose metadata is not loaded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ThreadRef {
    Article(u64),
    Missing(MessageId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadNode {
    pub article_number: u64,
    pub message_id: Option<MessageId>,
    /// The loaded parent article.
    pub parent: Option<u64>,
    /// The parent's message ID when it is not loaded. The article is then a
    /// child of that placeholder.
    pub unresolved_parent: Option<MessageId>,
    pub children: Vec<ThreadRef>,
}

/// A referenced article that is not loaded. Its parent comes from the
/// `References` of a loaded descendant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingNode {
    pub message_id: MessageId,
    pub parent: Option<ThreadRef>,
    pub children: Vec<ThreadRef>,
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
    pub missing: BTreeMap<MessageId, MissingNode>,
    pub roots: Vec<ThreadRef>,
    pub issues: Vec<ThreadIssue>,
}

impl ThreadTree {
    pub fn parent(&self, node: &ThreadRef) -> Option<ThreadRef> {
        match node {
            ThreadRef::Article(number) => {
                let node = self.nodes.get(number)?;
                node.parent.map(ThreadRef::Article).or_else(|| {
                    node.unresolved_parent
                        .as_ref()
                        .filter(|id| self.missing.contains_key(*id))
                        .cloned()
                        .map(ThreadRef::Missing)
                })
            }
            ThreadRef::Missing(id) => self.missing.get(id)?.parent.clone(),
        }
    }

    pub fn children(&self, node: &ThreadRef) -> &[ThreadRef] {
        match node {
            ThreadRef::Article(number) => self.nodes.get(number).map(|node| &node.children[..]),
            ThreadRef::Missing(id) => self.missing.get(id).map(|node| &node.children[..]),
        }
        .unwrap_or_default()
    }

    /// The root of the thread containing `node`.
    pub fn top(&self, node: &ThreadRef) -> ThreadRef {
        let mut current = node.clone();
        // Parent links are acyclic; the bound only guards against a corrupt
        // tree.
        for _ in 0..=self.nodes.len() + self.missing.len() {
            match self.parent(&current) {
                Some(parent) => current = parent,
                None => break,
            }
        }
        current
    }

    /// Every node below and including `root`, in depth-first order with
    /// children in tree order.
    pub fn walk(&self, root: &ThreadRef) -> Vec<ThreadRef> {
        let mut order = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(node) = pending.pop() {
            pending.extend(self.children(&node).iter().rev().cloned());
            order.push(node);
        }
        order
    }

    /// The loaded articles of the thread rooted at `root`, in walk order.
    pub fn walk_articles(&self, root: &ThreadRef) -> Vec<u64> {
        self.walk(root)
            .into_iter()
            .filter_map(|node| match node {
                ThreadRef::Article(number) => Some(number),
                ThreadRef::Missing(_) => None,
            })
            .collect()
    }

    /// The article that identifies a thread: the first loaded article in
    /// the walk from the thread's root.
    pub fn thread_first_article(&self, article_number: u64) -> Option<u64> {
        if !self.nodes.contains_key(&article_number) {
            return None;
        }
        let top = self.top(&ThreadRef::Article(article_number));
        self.walk_articles(&top).first().copied()
    }

    fn is_ancestor_or_self(&self, ancestor: &ThreadRef, node: &ThreadRef) -> bool {
        let mut current = Some(node.clone());
        for _ in 0..=self.nodes.len() + self.missing.len() {
            match current {
                Some(ref value) if value == ancestor => return true,
                Some(value) => current = self.parent(&value),
                None => return false,
            }
        }
        true
    }
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

    let mut tree = ThreadTree {
        nodes,
        missing: BTreeMap::new(),
        roots: Vec::new(),
        issues,
    };
    // Attach every article with an unloaded parent before chaining, so the
    // cycle checks below see all of those links.
    let unresolved = tree
        .nodes
        .values()
        .filter_map(|node| node.unresolved_parent.clone())
        .collect::<Vec<_>>();
    for id in unresolved {
        tree.missing
            .entry(id.clone())
            .or_insert_with(|| MissingNode {
                message_id: id,
                parent: None,
                children: Vec::new(),
            });
    }
    for article in &articles {
        add_missing_ancestors(&mut tree, article, &ids);
    }
    link_children(&mut tree);
    tree
}

/// Adds a placeholder for an article's unloaded parent, then chains
/// placeholders up through the earlier `References` IDs until reaching a
/// loaded article or a placeholder whose parent is already known. Earlier
/// articles take precedence, and no link may create a cycle.
fn add_missing_ancestors(
    tree: &mut ThreadTree,
    article: &ThreadArticle,
    ids: &BTreeMap<MessageId, Vec<u64>>,
) {
    let Some(parent_id) = tree
        .nodes
        .get(&article.article_number)
        .and_then(|node| node.unresolved_parent.clone())
    else {
        return;
    };

    // With `References`, the parent is its last entry; earlier entries are
    // older ancestors. An `In-Reply-To` parent has no known ancestors.
    let references = &article.metadata.references;
    if references.last() != Some(&parent_id) {
        return;
    }
    let mut current = parent_id;
    for ancestor_id in references[..references.len() - 1].iter().rev() {
        if tree.missing[&current].parent.is_some() {
            return;
        }
        let candidate = match ids.get(ancestor_id).and_then(|numbers| numbers.first()) {
            Some(number) => ThreadRef::Article(*number),
            None => ThreadRef::Missing(ancestor_id.clone()),
        };
        if let ThreadRef::Missing(id) = &candidate {
            tree.missing
                .entry(id.clone())
                .or_insert_with(|| MissingNode {
                    message_id: id.clone(),
                    parent: None,
                    children: Vec::new(),
                });
        }
        let child = ThreadRef::Missing(current.clone());
        if tree.is_ancestor_or_self(&child, &candidate) {
            return;
        }
        tree.missing
            .get_mut(&current)
            .expect("placeholder was inserted")
            .parent = Some(candidate.clone());
        match candidate {
            ThreadRef::Article(_) => return,
            ThreadRef::Missing(id) => current = id,
        }
    }
}

/// Fills in children and roots, each ordered by the lowest article number
/// at or below the node so placeholders sort beside the articles they hold.
fn link_children(tree: &mut ThreadTree) {
    let all = tree
        .nodes
        .keys()
        .map(|number| ThreadRef::Article(*number))
        .chain(tree.missing.keys().cloned().map(ThreadRef::Missing))
        .collect::<Vec<_>>();
    let mut children: HashMap<ThreadRef, Vec<ThreadRef>> = HashMap::new();
    let mut roots = Vec::new();
    for node in &all {
        match tree.parent(node) {
            Some(parent) => children.entry(parent).or_default().push(node.clone()),
            None => roots.push(node.clone()),
        }
    }

    let mut lowest: HashMap<ThreadRef, u64> = HashMap::new();
    for number in tree.nodes.keys() {
        let mut current = Some(ThreadRef::Article(*number));
        for _ in 0..=all.len() {
            let Some(node) = current else {
                break;
            };
            let entry = lowest.entry(node.clone()).or_insert(*number);
            *entry = (*entry).min(*number);
            current = tree.parent(&node);
        }
    }
    let sort_key = |node: &ThreadRef| match node {
        ThreadRef::Article(number) => (*number, 0),
        ThreadRef::Missing(_) => (lowest.get(node).copied().unwrap_or(u64::MAX), 1),
    };

    for (parent, mut list) in children {
        list.sort_by_key(|node| sort_key(node));
        match parent {
            ThreadRef::Article(number) => {
                if let Some(node) = tree.nodes.get_mut(&number) {
                    node.children = list;
                }
            }
            ThreadRef::Missing(id) => {
                if let Some(node) = tree.missing.get_mut(&id) {
                    node.children = list;
                }
            }
        }
    }
    // A placeholder with no loaded article below it is not shown as a root.
    roots.retain(|node| matches!(node, ThreadRef::Article(_)) || lowest.contains_key(node));
    roots.sort_by_key(|node| sort_key(node));
    tree.roots = roots;
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
        assert_eq!(tree.roots, [ThreadRef::Article(1), ThreadRef::Article(3)]);
        assert_eq!(tree.nodes[&1].children, [ThreadRef::Article(2)]);
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
        let missing = ThreadRef::Missing(MessageId::parse(b"<not-loaded>").unwrap());
        assert_eq!(tree.roots, [missing.clone()]);
        assert_eq!(tree.children(&missing), [ThreadRef::Article(12)]);
        assert_eq!(tree.parent(&ThreadRef::Article(12)), Some(missing));
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
        assert_eq!(tree.roots, [ThreadRef::Article(2)]);
        assert_eq!(tree.nodes[&2].children, [ThreadRef::Article(1)]);
        assert_eq!(
            tree.issues,
            [ThreadIssue::CycleBroken { article_number: 2 }]
        );
    }

    #[test]
    fn keeps_articles_without_message_ids_as_independent_roots() {
        let tree = build_threads([article(9, None, &[], None, None)]);
        assert_eq!(tree.roots, [ThreadRef::Article(9)]);
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

    fn id(value: &[u8]) -> MessageId {
        MessageId::parse(value).unwrap()
    }

    #[test]
    fn chains_placeholders_through_references_and_groups_orphans() {
        // Root and middle are not loaded; two replies to the middle and a
        // reply directly to the root share one thread.
        let tree = build_threads([
            article(5, Some(b"<a>"), &[b"<root>", b"<mid>"], None, None),
            article(7, Some(b"<b>"), &[b"<root>", b"<mid>"], None, None),
            article(9, Some(b"<c>"), &[b"<root>"], None, None),
        ]);
        let root = ThreadRef::Missing(id(b"<root>"));
        let mid = ThreadRef::Missing(id(b"<mid>"));
        assert_eq!(tree.roots, [root.clone()]);
        assert_eq!(tree.children(&root), [mid.clone(), ThreadRef::Article(9)]);
        assert_eq!(
            tree.children(&mid),
            [ThreadRef::Article(5), ThreadRef::Article(7)]
        );
        assert_eq!(tree.walk_articles(&root), [5, 7, 9]);
        assert_eq!(tree.top(&ThreadRef::Article(7)), root);
        assert_eq!(tree.thread_first_article(9), Some(5));
        assert_eq!(tree.nodes[&5].unresolved_parent, Some(id(b"<mid>")));
    }

    #[test]
    fn placeholder_chains_stop_at_a_loaded_ancestor() {
        let tree = build_threads([
            article(1, Some(b"<root>"), &[], None, None),
            article(4, Some(b"<reply>"), &[b"<root>", b"<gone>"], None, None),
        ]);
        let gone = ThreadRef::Missing(id(b"<gone>"));
        assert_eq!(tree.roots, [ThreadRef::Article(1)]);
        assert_eq!(tree.nodes[&1].children, [gone.clone()]);
        assert_eq!(tree.children(&gone), [ThreadRef::Article(4)]);
        assert_eq!(tree.walk_articles(&ThreadRef::Article(1)), [1, 4]);
    }

    #[test]
    fn in_reply_to_parents_get_a_placeholder_without_ancestors() {
        let tree = build_threads([article(3, Some(b"<x>"), &[], Some(b"<p>"), None)]);
        let p = ThreadRef::Missing(id(b"<p>"));
        assert_eq!(tree.roots, [p.clone()]);
        assert_eq!(tree.missing[&id(b"<p>")].parent, None);
    }

    #[test]
    fn conflicting_references_never_create_placeholder_cycles() {
        // 2 says <x> is under <y>; 3 says <y> is under <x>. The earlier
        // article wins and the later link is dropped.
        let tree = build_threads([
            article(2, Some(b"<two>"), &[b"<y>", b"<x>"], None, None),
            article(3, Some(b"<three>"), &[b"<x>", b"<y>"], None, None),
        ]);
        let x = ThreadRef::Missing(id(b"<x>"));
        let y = ThreadRef::Missing(id(b"<y>"));
        assert_eq!(tree.parent(&x), Some(y.clone()));
        assert_eq!(tree.parent(&y), None);
        assert_eq!(tree.roots, [y.clone()]);
        assert_eq!(tree.walk_articles(&y), [2, 3]);

        // A placeholder may not be placed under its own loaded child.
        let tree = build_threads([
            article(1, Some(b"<one>"), &[b"<p>"], None, None),
            article(2, Some(b"<two>"), &[b"<one>", b"<p>"], None, None),
        ]);
        let p = ThreadRef::Missing(id(b"<p>"));
        assert_eq!(tree.parent(&p), None);
        assert_eq!(tree.roots, [p.clone()]);
        assert_eq!(tree.walk_articles(&p), [1, 2]);
    }
}
