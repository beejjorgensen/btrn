use crossterm::cursor::Show;
use crossterm::terminal::{enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{execute, terminal};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

/// Whether the terminal is in raw mode on the alternate screen, so a panic
/// must restore it before the message is printed.
static TERMINAL_TAKEN: AtomicBool = AtomicBool::new(false);
static PANIC_HOOK: Once = Once::new();

/// Installs, once, a panic hook that restores the terminal before the
/// default hook prints the message, which would otherwise be drawn on the
/// alternate screen and lost when it is left.
fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if TERMINAL_TAKEN.swap(false, Ordering::SeqCst) {
                let _ = terminal::disable_raw_mode();
                let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
            }
            previous(info);
        }));
    });
}

pub(crate) struct TerminalSession {
    pub(crate) output: io::Stdout,
    raw_mode: bool,
    alternate_screen: bool,
}

impl TerminalSession {
    pub(crate) fn enter() -> io::Result<Self> {
        install_panic_hook();
        let mut output = io::stdout();
        execute!(output, EnterAlternateScreen)?;
        if let Err(error) = enable_raw_mode() {
            let _ = execute!(output, LeaveAlternateScreen);
            return Err(error);
        }
        TERMINAL_TAKEN.store(true, Ordering::SeqCst);
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
        TERMINAL_TAKEN.store(false, Ordering::SeqCst);
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
        TERMINAL_TAKEN.store(true, Ordering::SeqCst);
        if !self.raw_mode {
            enable_raw_mode()?;
            self.raw_mode = true;
        }
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        // During a panic the hook has already restored the terminal; leaving
        // the alternate screen again could move the cursor over the message.
        let taken = TERMINAL_TAKEN.swap(false, Ordering::SeqCst);
        if std::thread::panicking() && !taken {
            return;
        }
        if self.raw_mode {
            let _ = terminal::disable_raw_mode();
        }
        if self.alternate_screen {
            let _ = execute!(self.output, LeaveAlternateScreen);
        }
    }
}
