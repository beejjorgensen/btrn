use crossterm::cursor::{MoveTo, RestorePosition, SavePosition};
use crossterm::style::{Attribute, SetAttribute};
use crossterm::QueueableCommand;
use std::io::{self, Write};

pub(crate) fn selection_letter(index: usize) -> Option<char> {
    "abcdefghijklmnopqrstuvwxyz0123456789"
        .chars()
        .filter(|character| *character != 'q')
        .nth(index)
}

pub(crate) fn page_label(index: usize, count: usize, end_index: usize, total: usize) -> String {
    if count <= 1 {
        return "All".to_owned();
    }
    if index + 1 == count {
        return "Bot".to_owned();
    }
    let percent = if total == 0 {
        100
    } else {
        end_index.saturating_mul(100).div_ceil(total)
    };
    if index == 0 {
        format!("Top {percent}%")
    } else {
        format!("{percent}%")
    }
}

pub(crate) fn write_line(
    output: &mut impl Write,
    line: &str,
    terminate: bool,
    width: u16,
    inverse: bool,
) -> io::Result<()> {
    if inverse {
        output.queue(SetAttribute(Attribute::Reverse))?;
    }
    output.write_all(fit_terminal_line(line, width).as_bytes())?;
    if terminate {
        output.write_all(b"\r\n")?;
    }
    if inverse {
        output.queue(SetAttribute(Attribute::Reset))?;
    }
    Ok(())
}

pub(crate) fn write_mark(output: &mut impl Write, x: u16, y: u16, marked: bool) -> io::Result<()> {
    output.queue(SavePosition)?.queue(MoveTo(x, y))?;
    output.write_all(if marked { b"+" } else { b" " })?;
    output.queue(RestorePosition)?;
    output.flush()
}

pub(crate) fn fit_terminal_line(line: &str, width: u16) -> String {
    let sanitized = line
        .chars()
        .map(|character| {
            if character.is_control() {
                '\u{fffd}'
            } else {
                character
            }
        })
        .collect::<String>();
    let width = usize::from(width);
    if sanitized.len() <= width {
        return sanitized;
    }

    let mut fitted = String::new();
    let prefix_limit = width.saturating_sub('…'.len_utf8());
    for character in sanitized.chars() {
        if fitted.len() + character.len_utf8() > prefix_limit {
            break;
        }
        fitted.push(character);
    }
    if width > 0 {
        fitted.push('…');
    }
    fitted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_letters_skip_q() {
        assert_eq!(selection_letter(0), Some('a'));
        assert_eq!(selection_letter(16), Some('r'));
        assert_eq!(selection_letter(24), Some('z'));
        assert_eq!(selection_letter(25), Some('0'));
        assert_eq!(selection_letter(34), Some('9'));
        assert_eq!(selection_letter(35), None);
    }

    #[test]
    fn page_labels_match_selector_position() {
        assert_eq!(page_label(0, 1, 3, 3), "All");
        assert_eq!(page_label(0, 3, 2, 5), "Top 40%");
        assert_eq!(page_label(1, 3, 4, 5), "80%");
        assert_eq!(page_label(2, 3, 5, 5), "Bot");
    }

    #[test]
    fn line_writer_sanitizes_and_applies_inverse_attribute() {
        let mut output = Vec::new();
        write_line(&mut output, "ab\u{1b}c", false, 10, true).unwrap();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            "\u{1b}[7mab\u{fffd}c\u{1b}[0m"
        );
    }

    #[test]
    fn terminal_line_is_sanitized_and_fitted_to_columns() {
        assert_eq!(fit_terminal_line("ab\u{1b}cdef", 5), "ab…");
        assert_eq!(fit_terminal_line("a\u{1b}b", 10), "a�b");
        assert_eq!(fit_terminal_line("abcdef", 1), "…");
        assert_eq!(fit_terminal_line("okay", 10), "okay");
    }
}
