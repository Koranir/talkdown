//! Native file dialogs and asynchronous disk operations.
//!
//! Buffer-generation and conflict policy stay in `App`; this module only
//! adapts platform I/O into typed results for the update loop.

use super::FileObservation;

use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub(super) struct SavedFile {
    pub(super) path: PathBuf,
    pub(super) text: String,
    pub(super) revision: u64,
    pub(super) buffer_generation: u64,
}

#[derive(Debug, Clone)]
pub(super) enum FileError {
    DialogClosed,
    Io(io::ErrorKind),
}

impl std::fmt::Display for FileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DialogClosed => formatter.write_str("dialog closed"),
            Self::Io(kind) => write!(formatter, "I/O error: {kind}"),
        }
    }
}

pub(super) fn pick_file(
    window: &dyn iced::Window,
) -> impl Future<Output = Result<(PathBuf, String), FileError>> + use<> {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Open a text file…")
        .set_parent(&window);

    async move {
        let file = dialog.pick_file().await.ok_or(FileError::DialogClosed)?;
        let path = file.path().to_owned();
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| FileError::Io(error.kind()))?;
        Ok((path, contents))
    }
}

pub(super) async fn observe_file(path: PathBuf) -> FileObservation {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => FileObservation::Present(contents),
        Err(error) if error.kind() == io::ErrorKind::NotFound => FileObservation::Missing,
        Err(error) => FileObservation::Unreadable(error.kind()),
    }
}

pub(super) async fn save_to(
    path: PathBuf,
    text: String,
    revision: u64,
    buffer_generation: u64,
) -> Result<SavedFile, FileError> {
    tokio::fs::write(&path, text.as_bytes())
        .await
        .map_err(|error| FileError::Io(error.kind()))?;
    Ok(SavedFile {
        path,
        text,
        revision,
        buffer_generation,
    })
}

pub(super) fn pick_save_file(
    window: &dyn iced::Window,
    suggested: String,
    text: String,
    revision: u64,
    buffer_generation: u64,
) -> impl Future<Output = Result<SavedFile, FileError>> + use<> {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Save text file…")
        .set_file_name(suggested)
        .set_parent(&window);

    async move {
        let file = dialog.save_file().await.ok_or(FileError::DialogClosed)?;
        save_to(file.path().to_owned(), text, revision, buffer_generation).await
    }
}
