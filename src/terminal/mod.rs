pub mod article_view;
pub mod group_selector;
pub(crate) mod posting;
pub mod reader;
mod selector;
pub(crate) mod session;
mod thread_selector;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// Ctrl-C, which exits the program from any screen.
pub(crate) fn is_interrupt(key: &KeyEvent) -> bool {
    key.kind == KeyEventKind::Press
        && key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
}
