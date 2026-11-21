//! Plain-text command-line output for the non-interactive modes.

use crate::group_loader::LoadedGroupThreads;
use crate::state::SessionState;

pub fn format_thread_summary(loaded: &LoadedGroupThreads) -> String {
    let article_count = loaded
        .threads
        .rows
        .iter()
        .map(|row| row.article_numbers.len())
        .sum::<usize>();
    let unread_count = loaded
        .threads
        .rows
        .iter()
        .map(|row| row.unread_count)
        .sum::<usize>();
    let mut output = format!(
        "{}: {} threads, {} articles, {} unread\n",
        loaded.threads.group_name,
        loaded.threads.rows.len(),
        article_count,
        unread_count
    );
    for row in &loaded.threads.rows {
        let unresolved = if row.has_unresolved_parent {
            " (?)"
        } else {
            ""
        };
        output.push_str(&format!(
            "{}/{} {}{} {}\n",
            row.unread_count,
            row.article_numbers.len(),
            row.root_article_number,
            unresolved,
            row.subject
        ));
    }
    output
}

pub fn format_flat_overview(state: &SessionState, group: &str) -> String {
    let mut output = String::new();
    if let Some(group_state) = state.groups.get(group) {
        for (number, metadata) in &group_state.articles {
            let subject = metadata
                .subject
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            output.push_str(&format!("{number} {subject}\n"));
        }
    }
    output
}
