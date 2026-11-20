use crate::decoder::DecodedArticle;
use crate::state::{SessionState, StateError};
use crossterm::{
    cursor::MoveTo,
    queue,
    terminal::{Clear, ClearType},
};
use std::io::{self, Write};

pub fn render_article_view(
    output: &mut impl Write,
    group: &str,
    article_number: u64,
    article: &DecodedArticle,
) -> io::Result<()> {
    queue!(output, Clear(ClearType::All), MoveTo(0, 0))?;
    write!(output, "{group} - article {article_number}\r\n")?;
    for header in &article.headers {
        write_terminal_bytes(output, header.name.as_bytes(), true)?;
        output.write_all(b": ")?;
        write_terminal_bytes(output, header.value.as_bytes(), true)?;
        output.write_all(b"\r\n")?;
    }
    output.write_all(b"\r\n")?;
    if !article.body.is_empty() {
        for line in article.body.split('\n') {
            write_terminal_bytes(output, line.as_bytes(), false)?;
            output.write_all(b"\r\n")?;
        }
    }
    output.flush()
}

pub fn present_article(
    output: &mut impl Write,
    state: &mut SessionState,
    group: &str,
    article_number: u64,
    article: &DecodedArticle,
) -> Result<(), PresentArticleError> {
    render_article_view(output, group, article_number, article)
        .map_err(PresentArticleError::Render)?;
    state
        .mark_article_read(group, article_number)
        .map_err(PresentArticleError::State)
}

fn write_terminal_bytes(
    output: &mut impl Write,
    bytes: &[u8],
    tabs_as_spaces: bool,
) -> io::Result<()> {
    for character in String::from_utf8_lossy(bytes).chars() {
        if character == '\t' && tabs_as_spaces {
            output.write_all(b" ")?;
        } else if character.is_control() {
            output.write_all("\u{fffd}".as_bytes())?;
        } else {
            let mut encoded = [0; 4];
            output.write_all(character.encode_utf8(&mut encoded).as_bytes())?;
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum PresentArticleError {
    Render(io::Error),
    State(StateError),
}

impl std::fmt::Display for PresentArticleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Render(error) => write!(formatter, "could not render article: {error}"),
            Self::State(error) => write!(formatter, "could not update article read state: {error}"),
        }
    }
}

impl std::error::Error for PresentArticleError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decoder::{decode_article, DecodedArticle};
    use crate::model::article::{HeaderField, RawArticle};

    #[derive(Default)]
    struct TestWriter {
        bytes: Vec<u8>,
        fail_flush: bool,
    }

    impl Write for TestWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.fail_flush {
                Err(io::Error::other("flush failed"))
            } else {
                Ok(())
            }
        }
    }

    fn sample_article() -> DecodedArticle {
        let raw = RawArticle {
            headers: vec![HeaderField {
                name: b"Subject".to_vec(),
                value: b"hello\tthere\x1b[31m".to_vec(),
            }],
            body: vec![b"body".to_vec()],
        };
        decode_article(&raw)
    }

    #[test]
    fn marks_read_only_after_the_article_view_flushes_successfully() {
        let mut state = SessionState::default();
        let mut output = TestWriter::default();
        present_article(&mut output, &mut state, "comp.test", 9, &sample_article()).unwrap();

        assert!(state.is_read("comp.test", 9));
        let text = String::from_utf8(output.bytes).unwrap();
        assert!(!text
            .as_bytes()
            .windows(2)
            .any(|bytes| bytes[1] == b'\n' && bytes[0] != b'\r'));
        assert!(text.contains("comp.test"));
        assert!(text.contains("article 9"));
        assert!(text.contains("hello there\u{fffd}[31m"));
        assert!(text.contains("body"));
    }

    #[test]
    fn does_not_mark_read_when_render_or_flush_fails() {
        let mut state = SessionState::default();
        let mut output = TestWriter {
            fail_flush: true,
            ..TestWriter::default()
        };
        assert!(matches!(
            present_article(&mut output, &mut state, "comp.test", 9, &sample_article()),
            Err(PresentArticleError::Render(_))
        ));
        assert!(!state.is_read("comp.test", 9));
        assert!(state.newsrc.entries.is_empty());
    }
}
