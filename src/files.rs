use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// MAX_LISTED bounds one listing, so a folder of many files makes no huge response.
pub const MAX_LISTED: usize = 500;

/// MAX_UPLOAD caps one uploaded file at re:agent's own limit for an image it will open.
pub const MAX_UPLOAD: usize = 20 * 1024 * 1024;

/// Attachment is a file the user sent with a message, as saved in the chat's folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    pub path: String,
    pub size: u64,
}

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

/// Uploadable accepts a plain file name that the model's tools can see: not empty, without
/// a slash or control characters, and not hidden, since re:agent skips `.env` and `.git`.
pub fn uploadable(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

/// Save stores an upload at the folder's top level under its own name, or the first free
/// `name-2.ext`, `name-3.ext` and so on: an upload never replaces a file.
pub fn save(folder: &Path, name: &str, bytes: &[u8]) -> std::io::Result<Attachment> {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    for n in 1..=1000 {
        let path = match n {
            1 => name.to_owned(),
            n => format!("{stem}-{n}{extension}"),
        };
        let target = folder.join(&path);
        // Creating only a new file also refuses to follow a link someone left in its place.
        let mut file = match std::fs::File::options()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        if let Err(e) = file.write_all(bytes) {
            let _ = std::fs::remove_file(&target);
            return Err(e);
        }
        return Ok(Attachment {
            path,
            size: bytes.len() as u64,
        });
    }
    Err(std::io::Error::other(
        "a thousand files already have this name",
    ))
}

/// Kind names what a file is for the model: an image, text it can read, or something else.
pub fn kind(path: &Path) -> &'static str {
    const IMAGES: [&str; 10] = [
        "png", "jpg", "jpeg", "webp", "gif", "heic", "heif", "bmp", "tif", "tiff",
    ];
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if IMAGES.contains(&extension.to_ascii_lowercase().as_str()) {
        return "image";
    }
    let mut start = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| f.take(64 * 1024).read_to_end(&mut start));
    // A character cut by the 64 KiB window still counts as text.
    let text = match std::str::from_utf8(&start) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none(),
    };
    if read.is_ok() && text && !start.contains(&0) {
        "text"
    } else {
        "other"
    }
}
