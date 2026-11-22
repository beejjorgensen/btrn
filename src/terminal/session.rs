use crossterm::cursor::Show;
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

impl TerminalSession {
    /// Restores the normal screen and cooked mode so another program, such
    /// as the editor, can use the terminal.
    pub(crate) fn suspend(&mut self) -> io::Result<()> {
        if self.raw_mode {
            terminal::disable_raw_mode()?;
            self.raw_mode = false;
        }
        if self.alternate_screen {
            execute!(self.output, LeaveAlternateScreen, Show)?;
            self.alternate_screen = false;
        }
        Ok(())
    }

    /// Re-enters the alternate screen and raw mode after `suspend`.
    pub(crate) fn resume(&mut self) -> io::Result<()> {
        if !self.alternate_screen {
            execute!(self.output, EnterAlternateScreen)?;
            self.alternate_screen = true;
        }
        if !self.raw_mode {
            enable_raw_mode()?;
            self.raw_mode = true;
        }
        Ok(())
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
