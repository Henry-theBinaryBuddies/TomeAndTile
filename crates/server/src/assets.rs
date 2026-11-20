//! Asset storage for map and token images (ADR 0014).
//!
//! This module holds the configuration, the storage trait, the disk store and
//! the file-name rules. The HTTP routes are in `upload`. Files are content
//! addressed: `<64 lowercase hex sha-256>.<png|jpg|webp>`, so a name can never
//! contain a path separator and a cached copy can never be stale.
//!
//! # Asset upload (operator notes)
//!
//! * `TT_ASSET_DIR`: directory for uploads. Unset turns uploads OFF
//!   (`POST` answers 503 `uploads_off`, `GET /assets/..` answers 404).
//! * `TT_PUBLIC_URL`: optional `http://` or `https://` base for the `url` in
//!   the returned `AssetRef`; without it the request's `Host` header is used.
//! * `POST /campaigns/{campaign_id}/assets` with `Authorization: Bearer
//!   <token>` and the raw file as body. GM only. 200 with a protobuf
//!   `AssetRef`; errors are JSON `{"code","message"}`.
//! * `GET /assets/<id>.<ext>`: no auth, immutable cache headers.
//! * CORS: every response allows any origin.
//!
//! NOT SECURITY: with `TT_DEV_AUTH=1` anyone who can reach the server can
//! claim to be the GM and upload. Do not expose it to the internet.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tt_asset_pipeline::{inspect, MAX_FILE_BYTES};
use tt_contracts::v1::AssetRef;

/// Most files registered from the directory at start-up.
pub const MAX_REGISTERED_ASSETS: usize = 1000;
/// Most uploads being received or stored at once.
pub const MAX_CONCURRENT_UPLOADS: usize = 4;

/// Where uploads live and how their links are built.
#[derive(Debug, Clone)]
pub struct AssetConfig {
    /// Directory holding `<id>.<ext>` files. Must exist.
    pub dir: PathBuf,
    /// Base for asset links, no trailing slash. `None` uses the request's
    /// `Host` header (or the listener address at start-up).
    pub public_url: Option<String>,
}

/// File extension for a media type the server accepts.
pub fn ext_for_media_type(media_type: &str) -> Option<&'static str> {
    match media_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// Media type for a stored extension.
pub fn media_type_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "png" => Some("image/png"),
        "jpg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// The canonical form of an extension, or `None` if it is not accepted.
fn canonical_ext(ext: &str) -> Option<&'static str> {
    media_type_for_ext(ext).and_then(ext_for_media_type)
}

/// True for exactly 64 lowercase hex digits.
pub fn is_asset_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Split `<id>.<ext>` into its parts; `None` unless the whole name matches
/// `^[0-9a-f]{64}\.(png|jpg|webp)$`.
pub fn parse_file_name(name: &str) -> Option<(&str, &'static str)> {
    let (id, ext) = name.split_once('.')?;
    if !is_asset_id(id) {
        return None;
    }
    Some((id, canonical_ext(ext)?))
}

/// Where uploaded bytes live. Calls block; run them in `spawn_blocking`.
/// Cloudflare R2 can replace the disk store without touching the endpoint.
pub trait AssetStore: Send + Sync {
    /// Store `bytes` under `id` and `ext`. A file already there is kept.
    fn put(&self, id: &str, ext: &str, bytes: &[u8]) -> io::Result<()>;
    /// The stored bytes, or `None` if there is no such file.
    fn get(&self, id: &str, ext: &str) -> io::Result<Option<Vec<u8>>>;
    /// Whether the file is stored.
    fn exists(&self, id: &str, ext: &str) -> io::Result<bool>;
}

/// Files in one directory.
#[derive(Debug, Clone)]
pub struct DiskStore {
    dir: PathBuf,
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl DiskStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// The file path for a validated id and ext. Names are built only from a
    /// 64-hex id and a fixed extension, never from request text.
    fn path_for(&self, id: &str, ext: &str) -> io::Result<PathBuf> {
        match (is_asset_id(id), canonical_ext(ext)) {
            (true, Some(ext)) => Ok(self.dir.join(format!("{id}.{ext}"))),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a valid asset name",
            )),
        }
    }
}

impl AssetStore for DiskStore {
    fn put(&self, id: &str, ext: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.path_for(id, ext)?;
        if path.is_file() {
            return Ok(());
        }
        // Write beside the target, then rename: readers never see half a file.
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp = self
            .dir
            .join(format!(".{id}.{}.{n}.tmp", std::process::id()));
        let result = fs::write(&temp, bytes).and_then(|()| fs::rename(&temp, &path));
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    fn get(&self, id: &str, ext: &str) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path_for(id, ext)?) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn exists(&self, id: &str, ext: &str) -> io::Result<bool> {
        Ok(self.path_for(id, ext)?.is_file())
    }
}

/// Read the directory once and return each valid stored image (with an empty
/// `url`) and its extension. A file counts only if its name is
/// `<64 hex>.<ext>`, its bytes pass the pipeline, its hash equals the name
/// and its extension matches its format. Anything else is skipped with a note
/// on stderr. At most [`MAX_REGISTERED_ASSETS`] matching names are read.
pub fn scan_dir(dir: &Path) -> Vec<(AssetRef, &'static str)> {
    let mut found = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("tt-server: cannot read asset dir {}: {e}", dir.display());
            return found;
        }
    };
    let mut candidates = 0usize;
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        let Some((id, ext)) = parse_file_name(&name) else {
            eprintln!("tt-server: skipping {name:?} in the asset dir: not an asset name");
            continue;
        };
        if candidates >= MAX_REGISTERED_ASSETS {
            eprintln!(
                "tt-server: asset dir has more than {MAX_REGISTERED_ASSETS} files; \
                 the rest are not registered"
            );
            break;
        }
        candidates += 1;
        let unusable = entry
            .metadata()
            .map(|m| !m.is_file() || m.len() > MAX_FILE_BYTES)
            .unwrap_or(true);
        if unusable {
            eprintln!("tt-server: skipping {name:?}: not a regular file or too large");
            continue;
        }
        let bytes = match fs::read(entry.path()) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("tt-server: skipping {name:?}: {e}");
                continue;
            }
        };
        match inspect(&bytes) {
            Ok(asset)
                if asset.asset_id == id && ext_for_media_type(&asset.media_type) == Some(ext) =>
            {
                found.push((asset, ext));
            }
            Ok(_) => eprintln!("tt-server: skipping {name:?}: name does not match contents"),
            Err(e) => eprintln!("tt-server: skipping {name:?}: {}", e.code()),
        }
    }
    found
}

/// Caps how many uploads run at once. Not a queue: when full, say so.
#[derive(Debug, Clone)]
pub struct UploadGate {
    permits: Arc<Semaphore>,
}

impl UploadGate {
    pub fn new(limit: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(limit)),
        }
    }

    /// A slot held until the returned permit is dropped, or `None` if all
    /// are taken (the route answers 503 `busy`).
    pub fn try_enter(&self) -> Option<OwnedSemaphorePermit> {
        self.permits.clone().try_acquire_owned().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn file_names_are_strict() {
        assert_eq!(parse_file_name(&format!("{ID}.png")), Some((ID, "png")));
        assert_eq!(parse_file_name(&format!("{ID}.jpg")), Some((ID, "jpg")));
        assert_eq!(parse_file_name(&format!("{ID}.webp")), Some((ID, "webp")));
        for bad in [
            format!("{}.png", ID.to_uppercase()),
            format!("{}.png", &ID[1..]),
            format!("{ID}.gif"),
            format!("{ID}.PNG"),
            format!("{ID}.png.png"),
            format!("..%2F{ID}.png"),
            format!("../{ID}.png"),
            ID.to_string(),
            String::new(),
        ] {
            assert_eq!(parse_file_name(&bad), None, "{bad}");
        }
    }

    #[test]
    fn busy_gate_refuses_the_fifth_upload_and_frees_on_drop() {
        let gate = UploadGate::new(MAX_CONCURRENT_UPLOADS);
        let held: Vec<_> = (0..MAX_CONCURRENT_UPLOADS)
            .map(|_| gate.try_enter().expect("slot free"))
            .collect();
        assert!(gate.try_enter().is_none());
        drop(held);
        assert!(gate.try_enter().is_some());
    }

    #[test]
    fn disk_store_refuses_bad_names() {
        let store = DiskStore::new(std::env::temp_dir());
        assert!(store.put("../x", "png", b"a").is_err());
        assert!(store.get(ID, "exe").is_err());
        assert!(store.exists("short", "png").is_err());
    }
}
