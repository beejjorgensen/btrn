//! The temporary composition file and the external editor. This is the only
//! place an article body (the quoted text) is written to disk, and the file
//! is removed when this value is dropped.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct CompositionFile {
    path: PathBuf,
}

impl CompositionFile {
    /// Creates a new file readable and writable only by the user, with a
    /// random name in the temporary directory (`$TMPDIR` when set).
    pub fn create(contents: &str) -> io::Result<Self> {
        Self::create_in(&std::env::temp_dir(), contents)
    }

    pub fn create_in(directory: &Path, contents: &str) -> io::Result<Self> {
        let path = directory.join(format!("btrn-{}.txt", super::random_token()));
        let mut file = private_file(&path)?;
        let composition = Self { path };
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        Ok(composition)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read(&self) -> io::Result<Vec<u8>> {
        fs::read(&self.path)
    }
}

impl Drop for CompositionFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn private_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[derive(Debug)]
pub enum EditorError {
    /// No editor command is configured.
    NoCommand,
    Launch(io::Error),
    /// The editor exited unsuccessfully; `None` when killed by a signal.
    Failed(Option<i32>),
}

impl std::fmt::Display for EditorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCommand => write!(formatter, "no editor is configured"),
            Self::Launch(error) => write!(formatter, "could not start the editor: {error}"),
            Self::Failed(Some(code)) => write!(formatter, "the editor exited with status {code}"),
            Self::Failed(None) => write!(formatter, "the editor was terminated by a signal"),
        }
    }
}

impl std::error::Error for EditorError {}

/// Runs `command` (program, then arguments) on `path` and waits for it. The
/// program is found through `PATH`; no shell is used.
pub fn run_editor(command: &[String], path: &Path) -> Result<(), EditorError> {
    let (program, arguments) = command
        .split_first()
        .filter(|(program, _)| !program.is_empty())
        .ok_or(EditorError::NoCommand)?;
    let status = Command::new(program)
        .args(arguments)
        .arg(path)
        .status()
        .map_err(EditorError::Launch)?;
    if status.success() {
        Ok(())
    } else {
        Err(EditorError::Failed(status.code()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("btrn-compose-test-{name}-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn creates_a_private_file_and_deletes_it_on_drop() {
        let directory = scratch_dir("lifecycle");
        let file = CompositionFile::create_in(&directory, "hello\n").unwrap();
        let path = file.path().to_owned();
        assert_eq!(file.read().unwrap(), b"hello\n");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("btrn-") && name.ends_with(".txt"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(file);
        assert!(!path.exists());
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn runs_the_editor_on_the_file_and_reports_failures() {
        let directory = scratch_dir("editor");
        let file = CompositionFile::create_in(&directory, "draft\n").unwrap();
        run_editor(
            &["sh".into(), "-c".into(), "printf edited > \"$0\"".into()],
            file.path(),
        )
        .unwrap();
        assert_eq!(file.read().unwrap(), b"edited");

        assert!(matches!(
            run_editor(&["false".into()], file.path()),
            Err(EditorError::Failed(Some(1)))
        ));
        assert!(matches!(
            run_editor(&["btrn-no-such-editor".into()], file.path()),
            Err(EditorError::Launch(_))
        ));
        assert!(matches!(
            run_editor(&[], file.path()),
            Err(EditorError::NoCommand)
        ));
        drop(file);
        fs::remove_dir_all(&directory).unwrap();
    }
}
