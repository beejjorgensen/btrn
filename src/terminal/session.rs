use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, terminal};
use std::io;

pub(crate) struct TerminalSession {
    pub(crate) output: io::Stdout,
    raw_mode: bool,
    alternate_screen: bool,
}

impl TerminalSession {
    pub(crate) fn enter() -> io::Result<Self> {
        let mut output = io::stdout();
        execute!(output, EnterAlternateScreen)?;
        if let Err(error) = enable_raw_mode() {
            let _ = execute!(output, LeaveAlternateScreen);
            return Err(error);
        }
        Ok(Self {
            output,
            raw_mode: true,
            alternate_screen: true,
        })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if self.raw_mode {
            let _ = terminal::disable_raw_mode();
        }
        if self.alternate_screen {
            let _ = execute!(self.output, LeaveAlternateScreen);
        }
    }
}
