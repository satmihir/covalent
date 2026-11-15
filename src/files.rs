use serde::Serialize;
use std::path::{Path, PathBuf};

/// MAX_LISTED bounds one listing, so a folder of many files makes no huge response.
pub const MAX_LISTED: usize = 500;

/// File is one file in a chat's folder, as the files panel lists it.
#[derive(Debug, Serialize)]
pub struct File {
    /// path is relative to the folder, with `/` between its parts.
    pub path: String,
    pub size: u64,
    pub modified_at_ms: i64,
}

/// Listing is a folder's files in path order.
#[derive(Debug, Serialize)]
pub struct Listing {
    pub files: Vec<File>,
    /// more says files after the first MAX_LISTED were left out.
    pub more: bool,
}

/// List finds the files in a folder and the folders inside it. It does not follow symbolic
/// links, which may point outside, and skips names that are not UTF-8, which no URL can carry.
pub fn list(folder: &Path) -> std::io::Result<Listing> {
    let mut files = Vec::new();
    let mut folders = vec![String::new()];
    while let Some(prefix) = folders.pop() {
        for entry in std::fs::read_dir(folder.join(&prefix))? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            let kind = entry.file_type()?;
            if kind.is_dir() {
                folders.push(path);
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let metadata = entry.metadata()?;
            let modified_at_ms = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as i64);
            files.push(File {
                path,
                size: metadata.len(),
                modified_at_ms,
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let more = files.len() > MAX_LISTED;
    files.truncate(MAX_LISTED);
    Ok(Listing { files, more })
}

/// Inside resolves a relative path within a folder, refusing one that leaves it, through `..`
/// or a symbolic link, and anything that is not a file.
pub fn inside(folder: &Path, relative: &str) -> Option<PathBuf> {
    let folder = folder.canonicalize().ok()?;
    let path = folder.join(relative).canonicalize().ok()?;
    (path.starts_with(&folder) && path.is_file()).then_some(path)
}
