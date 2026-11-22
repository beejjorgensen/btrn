//! The followup posting screens: posting check, warning, editor hand-off,
//! and the finalization menu. Decisions are made by pure functions; the
//! flow function only renders, reads keys, and calls the editor and NNTP.

use crate::decoder::DecodedArticle;
use crate::nntp::active::PostingStatus;
use crate::nntp::{NntpSession, ServerCapabilities};
use crate::posting::compose_file::{run_editor, CompositionFile, EditorError};
use crate::posting::composition::{parse_composition, Composition};
use crate::posting::draft::compose_followup;
use crate::posting::outgoing::{build_article, format_date, generate_message_id, OutgoingContext};
use crate::posting::PostingSettings;
use crate::terminal::is_interrupt;
use crate::terminal::session::TerminalSession;
use crossterm::cursor::{MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::queue;
use crossterm::terminal::{self, Clear, ClearType};
use std::io::{self, Write};
use std::time::SystemTime;

const WARNING_PARAGRAPHS: [&str; 2] = [
    "(Be sure to double-check the attribution against the signature, and trim \
     the quoted article down as much as possible.)",
    "This program posts news to thousands of machines throughout the entire \
     civilized world.  Your message will cost the net hundreds if not \
     thousands of dollars to send everywhere.  Please be sure you know what \
     you are doing.",
];
const POSTER_PARAGRAPH: &str = "The author requested replies by email (Followup-To: poster).";
const WARNING_QUESTION: &str = "Are you absolutely sure that you want to do this? [ny]";
const UNCHANGED_WARNING: &str = "Warning: the article was not changed.";
const CONTINUE_HINT: &str = "Press any key to continue.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostingExit {
    /// Back to the end-of-article prompt of the followed-up-on article.
    ReturnToArticle,
    /// Ctrl-C: exit the program.
    QuitApplication,
}

/// Why posting is refused, if it is.
pub(crate) fn posting_refusal(
    greeting_allows_posting: bool,
    capabilities: Option<&ServerCapabilities>,
    group: &str,
    group_status: Option<&PostingStatus>,
) -> Option<String> {
    if !greeting_allows_posting {
        return Some("This server does not allow posting.".to_owned());
    }
    if capabilities.is_some_and(|capabilities| !capabilities.supports("POST")) {
        return Some("This server does not support posting.".to_owned());
    }
    if group_status == Some(&PostingStatus::Prohibited) {
        return Some(format!("Posting to {group} is not allowed."));
    }
    None
}

/// The warning screen's paragraphs, ending with the question.
pub(crate) fn warning_paragraphs(followup_to_poster: bool) -> Vec<&'static str> {
    let mut paragraphs = WARNING_PARAGRAPHS.to_vec();
    if followup_to_poster {
        paragraphs.push(POSTER_PARAGRAPH);
    }
    paragraphs.push(WARNING_QUESTION);
    paragraphs
}

/// Word-wraps a paragraph at `width` columns, keeping the original spacing
/// between words on a line and dropping it at line breaks. A word longer
/// than the width is broken.
pub(crate) fn wrap_paragraph(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut rest = text;
    loop {
        let separator_len = rest.len() - rest.trim_start_matches(' ').len();
        let separator = &rest[..separator_len];
        rest = &rest[separator_len..];
        if rest.is_empty() {
            break;
        }
        let word_len = rest.find(' ').unwrap_or(rest.len());
        let mut word = &rest[..word_len];
        rest = &rest[word_len..];
        let columns = |text: &str| text.chars().count();
        if !line.is_empty() && columns(&line) + columns(separator) + columns(word) <= width {
            line.push_str(separator);
            line.push_str(word);
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        while columns(word) > width {
            let split = word
                .char_indices()
                .nth(width)
                .map_or(word.len(), |(index, _)| index);
            lines.push(word[..split].to_owned());
            word = &word[split..];
        }
        line.push_str(word);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Menu {
    SendEditAbort,
    EditAbort,
}

impl Menu {
    fn prompt(self) -> &'static str {
        match self {
            Self::SendEditAbort => "Send, Edit, or Abort [sea]?",
            Self::EditAbort => "Edit or Abort [ea]?",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuChoice {
    Send,
    Edit,
    Abort,
}

/// The menu choice for a key; invalid keys (including `Esc`, and `s` when
/// sending is not offered) do nothing.
pub(crate) fn menu_choice(menu: Menu, key: &KeyEvent) -> Option<MenuChoice> {
    if key.kind != KeyEventKind::Press
        || key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    match (menu, key.code) {
        (Menu::SendEditAbort, KeyCode::Char('s')) => Some(MenuChoice::Send),
        (_, KeyCode::Char('e')) => Some(MenuChoice::Edit),
        (_, KeyCode::Char('a')) => Some(MenuChoice::Abort),
        _ => None,
    }
}

/// Whether the warning screen's answer is `y` (case-sensitive).
pub(crate) fn confirms_posting(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('y')
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

/// What to show after the editor exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AfterEdit {
    /// The editor could not be started: show the message, then return to
    /// the article.
    LaunchFailed(String),
    Menu {
        messages: Vec<String>,
        menu: Menu,
        /// The checked composition, present only when it can be sent.
        composition: Option<Composition>,
    },
}

pub(crate) fn after_edit(
    editor: Result<(), EditorError>,
    before: &[u8],
    after: io::Result<Vec<u8>>,
) -> AfterEdit {
    match editor {
        Err(error @ (EditorError::NoCommand | EditorError::Launch(_))) => {
            return AfterEdit::LaunchFailed(format!("ERROR: {error}."));
        }
        Err(error @ EditorError::Failed(_)) => {
            return AfterEdit::Menu {
                messages: vec![format!("ERROR: {error}.")],
                menu: Menu::EditAbort,
                composition: None,
            };
        }
        Ok(()) => {}
    }
    let contents = match after {
        Ok(contents) => contents,
        Err(error) => {
            return AfterEdit::Menu {
                messages: vec![format!("ERROR: could not read the article: {error}.")],
                menu: Menu::EditAbort,
                composition: None,
            }
        }
    };
    let mut messages = Vec::new();
    if contents == before {
        messages.push(UNCHANGED_WARNING.to_owned());
    }
    let parsed = parse_composition(&contents);
    messages.extend(parsed.errors.iter().map(ToString::to_string));
    if parsed.errors.is_empty() {
        AfterEdit::Menu {
            messages,
            menu: Menu::SendEditAbort,
            composition: Some(parsed.composition),
        }
    } else {
        AfterEdit::Menu {
            messages,
            menu: Menu::EditAbort,
            composition: None,
        }
    }
}

/// The finalization screen's lines: messages with a blank line above and
/// below, then the menu prompt.
pub(crate) fn menu_lines(messages: &[String], menu: Menu, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    if !messages.is_empty() {
        lines.push(String::new());
        for message in messages {
            lines.extend(wrap_paragraph(message, width));
        }
    }
    lines.push(String::new());
    lines.push(menu.prompt().to_owned());
    lines
}

/// The warning screen's lines; the cursor goes after the last one.
pub(crate) fn warning_lines(followup_to_poster: bool, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, paragraph) in warning_paragraphs(followup_to_poster).iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(wrap_paragraph(paragraph, width));
    }
    lines
}

fn message_lines(messages: &[String], width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for message in messages {
        lines.extend(wrap_paragraph(message, width));
    }
    lines.push(String::new());
    lines.push(CONTINUE_HINT.to_owned());
    lines
}

/// Draws `lines` on a cleared screen with the cursor after the last line,
/// separated by a space.
fn render_lines(output: &mut impl Write, lines: &[String]) -> io::Result<()> {
    queue!(output, Clear(ClearType::All), MoveTo(0, 0), Show)?;
    let sanitized = lines
        .iter()
        .map(|line| {
            line.chars()
                .map(|character| {
                    if character.is_control() {
                        '\u{fffd}'
                    } else {
                        character
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    output.write_all(sanitized.join("\r\n").as_bytes())?;
    output.write_all(b" ")?;
    output.flush()
}

/// Renders a screen and waits for a key press, re-rendering on resize.
fn prompt(
    terminal: &mut TerminalSession,
    lines: impl Fn(usize) -> Vec<String>,
) -> io::Result<KeyEvent> {
    loop {
        let (width, _) = terminal::size()?;
        render_lines(&mut terminal.output, &lines(usize::from(width)))?;
        loop {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => return Ok(key),
                Event::Resize(_, _) => break,
                _ => {}
            }
        }
    }
}

/// Shows messages until a key is pressed.
fn show_messages(terminal: &mut TerminalSession, messages: &[String]) -> io::Result<PostingExit> {
    let key = prompt(terminal, |width| message_lines(messages, width))?;
    Ok(if is_interrupt(&key) {
        PostingExit::QuitApplication
    } else {
        PostingExit::ReturnToArticle
    })
}

/// Runs the `F` followup flow for `article` in `group`.
pub(crate) fn run_followup(
    session: &mut NntpSession,
    terminal: &mut TerminalSession,
    settings: &PostingSettings,
    group: &str,
    article: &DecodedArticle,
) -> io::Result<PostingExit> {
    let checks_group = session.posting_allowed()
        && session
            .server_capabilities()
            .is_none_or(|capabilities| capabilities.supports("POST"));
    let group_status = if checks_group {
        match session.group_posting_status(group) {
            Ok(status) => status,
            Err(error) => {
                return show_messages(
                    terminal,
                    &[format!(
                        "ERROR: could not check whether posting is allowed: {error}."
                    )],
                )
            }
        }
    } else {
        None
    };
    if let Some(refusal) = posting_refusal(
        session.posting_allowed(),
        session.server_capabilities(),
        group,
        group_status.as_ref(),
    ) {
        return show_messages(terminal, &[refusal]);
    }

    let draft = compose_followup(article, &settings.identity);
    let key = prompt(terminal, |width| {
        warning_lines(draft.followup_to_poster, width)
    })?;
    if is_interrupt(&key) {
        return Ok(PostingExit::QuitApplication);
    }
    if !confirms_posting(&key) {
        return Ok(PostingExit::ReturnToArticle);
    }

    let file = match CompositionFile::create(&draft.text) {
        Ok(file) => file,
        Err(error) => {
            return show_messages(
                terminal,
                &[format!(
                    "ERROR: could not create the article file: {error}."
                )],
            )
        }
    };
    let mut message_id: Option<String> = None;
    loop {
        let before = file.read().unwrap_or_default();
        terminal.suspend()?;
        let editor = run_editor(&settings.editor, file.path());
        terminal.resume()?;
        let (mut messages, mut menu, composition) = match after_edit(editor, &before, file.read()) {
            AfterEdit::LaunchFailed(message) => return show_messages(terminal, &[message]),
            AfterEdit::Menu {
                messages,
                menu,
                composition,
            } => (messages, menu, composition),
        };
        loop {
            let key = prompt(terminal, |width| menu_lines(&messages, menu, width))?;
            if is_interrupt(&key) {
                return Ok(PostingExit::QuitApplication);
            }
            match menu_choice(menu, &key) {
                None => {}
                Some(MenuChoice::Abort) => return Ok(PostingExit::ReturnToArticle),
                Some(MenuChoice::Edit) => break,
                Some(MenuChoice::Send) => {
                    let composition = composition
                        .as_ref()
                        .expect("Send is only offered for a checked composition");
                    let from = composition
                        .from()
                        .expect("checked composition has a valid From");
                    let message_id = message_id
                        .get_or_insert_with(|| generate_message_id(&from))
                        .clone();
                    let date = format_date(SystemTime::now());
                    let article = build_article(
                        composition,
                        &OutgoingContext {
                            references: &draft.references,
                            message_id: &message_id,
                            date: &date,
                        },
                    );
                    render_lines(&mut terminal.output, &["Posting...".to_owned()])?;
                    match session.post(&article) {
                        Ok(()) => return Ok(PostingExit::ReturnToArticle),
                        Err(error) => {
                            messages = vec![format!("ERROR: {error}")];
                            menu = Menu::SendEditAbort;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn capabilities(lines: &[&str]) -> ServerCapabilities {
        ServerCapabilities::parse(
            &lines
                .iter()
                .map(|line| line.as_bytes().to_vec())
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn refuses_posting_for_each_server_signal() {
        let with_post = capabilities(&["READER", "POST"]);
        let without_post = capabilities(&["READER"]);
        assert_eq!(
            posting_refusal(false, Some(&with_post), "a.b", None).as_deref(),
            Some("This server does not allow posting.")
        );
        assert_eq!(
            posting_refusal(true, Some(&without_post), "a.b", None).as_deref(),
            Some("This server does not support posting.")
        );
        assert_eq!(
            posting_refusal(
                true,
                Some(&with_post),
                "a.b",
                Some(&PostingStatus::Prohibited)
            )
            .as_deref(),
            Some("Posting to a.b is not allowed.")
        );
        assert_eq!(posting_refusal(true, Some(&with_post), "a.b", None), None);
        assert_eq!(
            posting_refusal(true, None, "a.b", Some(&PostingStatus::Moderated)),
            None
        );
    }

    #[test]
    fn warning_wraps_cleanly_and_ends_with_the_question() {
        let lines = warning_lines(false, 40);
        assert!(lines.iter().all(|line| line.chars().count() <= 40));
        assert_eq!(lines[0], "(Be sure to double-check the attribution");
        assert_eq!(lines.last().unwrap(), "do this? [ny]");
        let blank = lines.iter().filter(|line| line.is_empty()).count();
        assert_eq!(blank, 2);
        assert_eq!(
            warning_lines(false, 500)[2],
            WARNING_PARAGRAPHS[1],
            "spacing between sentences is preserved"
        );
        assert!(warning_lines(true, 200).contains(&POSTER_PARAGRAPH.to_owned()));
    }

    #[test]
    fn wraps_long_words_and_empty_text() {
        assert_eq!(wrap_paragraph("abcdefgh ij", 3), ["abc", "def", "gh", "ij"]);
        assert_eq!(wrap_paragraph("", 10), [""]);
        assert_eq!(wrap_paragraph("a  b", 10), ["a  b"]);
    }

    #[test]
    fn only_lowercase_y_confirms() {
        assert!(confirms_posting(&key(KeyCode::Char('y'))));
        assert!(!confirms_posting(&key(KeyCode::Char('Y'))));
        assert!(!confirms_posting(&key(KeyCode::Char('n'))));
        assert!(!confirms_posting(&key(KeyCode::Enter)));
    }

    #[test]
    fn menu_keys_depend_on_whether_send_is_offered() {
        assert_eq!(
            menu_choice(Menu::SendEditAbort, &key(KeyCode::Char('s'))),
            Some(MenuChoice::Send)
        );
        assert_eq!(menu_choice(Menu::EditAbort, &key(KeyCode::Char('s'))), None);
        assert_eq!(
            menu_choice(Menu::EditAbort, &key(KeyCode::Char('e'))),
            Some(MenuChoice::Edit)
        );
        assert_eq!(
            menu_choice(Menu::EditAbort, &key(KeyCode::Char('a'))),
            Some(MenuChoice::Abort)
        );
        assert_eq!(menu_choice(Menu::SendEditAbort, &key(KeyCode::Esc)), None);
        assert_eq!(
            menu_choice(Menu::SendEditAbort, &key(KeyCode::Char('x'))),
            None
        );
        let control_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(is_interrupt(&control_c));
        assert_eq!(menu_choice(Menu::SendEditAbort, &control_c), None);
    }

    const VALID: &[u8] = b"Newsgroups: a.b\nSubject: s\nFrom: u@example.com\n\nbody\n";

    #[test]
    fn editor_launch_failure_returns_to_the_article() {
        let outcome = after_edit(
            Err(EditorError::Launch(io::Error::from(
                io::ErrorKind::NotFound,
            ))),
            VALID,
            Ok(VALID.to_vec()),
        );
        assert!(
            matches!(outcome, AfterEdit::LaunchFailed(message) if message.starts_with("ERROR: could not start the editor"))
        );
    }

    #[test]
    fn editor_failure_offers_only_edit_or_abort() {
        assert_eq!(
            after_edit(Err(EditorError::Failed(Some(2))), VALID, Ok(b"x".to_vec())),
            AfterEdit::Menu {
                messages: vec!["ERROR: the editor exited with status 2.".to_owned()],
                menu: Menu::EditAbort,
                composition: None,
            }
        );
    }

    #[test]
    fn unchanged_file_warns_and_still_checks_headers() {
        let AfterEdit::Menu {
            messages,
            menu,
            composition,
        } = after_edit(Ok(()), VALID, Ok(VALID.to_vec()))
        else {
            panic!("expected the menu");
        };
        assert_eq!(messages, [UNCHANGED_WARNING]);
        assert_eq!(menu, Menu::SendEditAbort);
        assert!(composition.is_some());

        let invalid = b"Subject: s\n\nbody\n";
        let AfterEdit::Menu { messages, menu, .. } =
            after_edit(Ok(()), invalid, Ok(invalid.to_vec()))
        else {
            panic!("expected the menu");
        };
        assert_eq!(messages[0], UNCHANGED_WARNING);
        assert!(messages[1].starts_with("ERROR: the \"Newsgroups:\" line"));
        assert_eq!(menu, Menu::EditAbort);
    }

    #[test]
    fn edited_valid_file_offers_send() {
        assert!(matches!(
            after_edit(Ok(()), b"old", Ok(VALID.to_vec())),
            AfterEdit::Menu {
                ref messages,
                menu: Menu::SendEditAbort,
                composition: Some(_),
            } if messages.is_empty()
        ));
    }

    #[test]
    fn menu_screen_puts_messages_between_blank_lines_above_the_prompt() {
        assert_eq!(
            menu_lines(
                &["ERROR: one.".to_owned(), "ERROR: two.".to_owned()],
                Menu::EditAbort,
                80
            ),
            ["", "ERROR: one.", "ERROR: two.", "", "Edit or Abort [ea]?"]
        );
        assert_eq!(
            menu_lines(&[], Menu::SendEditAbort, 80),
            ["", "Send, Edit, or Abort [sea]?"]
        );
    }

    #[test]
    fn rendering_places_the_cursor_after_the_prompt_and_sanitizes() {
        let mut output = Vec::new();
        render_lines(
            &mut output,
            &["one\u{1b}".to_owned(), "two [ny]".to_owned()],
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.ends_with("one\u{fffd}\r\ntwo [ny] "));
    }
}
