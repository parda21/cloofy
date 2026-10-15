//! The library, Home and the open track list as the last run left them, kept on disk so the
//! next start shows them at once while fresh data loads.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use cloofy_spotify::{
    catalog::{AlbumInfo, ArtistInfo, PlaylistInfo, ShowInfo, Track, User},
    home::Home,
    paths::write_private,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Raised whenever the format changes; files of another version are ignored.
const VERSION: u32 = 1;
/// Older data would mislead more than it helps.
const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Serialize, Deserialize)]
pub struct Library {
    pub user: User,
    pub playlists: Vec<PlaylistInfo>,
    pub albums: Vec<AlbumInfo>,
    pub artists: Vec<ArtistInfo>,
    pub shows: Vec<ShowInfo>,
    pub liked_total: Option<u32>,
    pub home: Option<Home>,
}

/// The track list that was open.
#[derive(Serialize, Deserialize)]
pub struct Tracks {
    /// Which list it is: Liked Songs, or a playlist or album URI.
    pub key: String,
    pub total: u32,
    pub tracks: Vec<Track>,
}

#[derive(Serialize, Deserialize)]
struct File<T> {
    version: u32,
    /// Unix time in seconds.
    saved_at: u64,
    data: T,
}

#[derive(Deserialize)]
struct Header {
    version: u32,
}

pub fn library_path(cache: &Path) -> PathBuf {
    cache.join("library.json")
}

pub fn tracks_path(cache: &Path) -> PathBuf {
    cache.join("tracks.json")
}

/// Cached data and how old it is; `None` when missing, unreadable, of another version or
/// too old.
pub fn load<T: DeserializeOwned>(path: &Path) -> Option<(T, Duration)> {
    let bytes = fs::read(path).ok()?;
    // The version decides how to read the rest, so it is checked on its own first.
    let header: Header = serde_json::from_slice(&bytes).ok()?;
    if header.version != VERSION {
        return None;
    }
    let file: File<T> = serde_json::from_slice(&bytes).ok()?;
    let age = Duration::from_secs(now().saturating_sub(file.saved_at));
    (age <= MAX_AGE).then_some((file.data, age))
}

pub fn save<T: Serialize>(path: &Path, data: &T) -> io::Result<()> {
    let file = File { version: VERSION, saved_at: now(), data };
    // Write and rename, so a crash mid-write never leaves a broken file behind.
    let temp = path.with_extension("json.tmp");
    write_private(&temp, &serde_json::to_vec(&file)?)?;
    fs::rename(temp, path)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cloofy-datacache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn reads_back_what_it_saved() {
        let dir = dir("roundtrip");
        let path = tracks_path(&dir);
        let tracks = Tracks { key: "liked".into(), total: 0, tracks: Vec::new() };
        save(&path, &tracks).unwrap();
        let (loaded, age) = load::<Tracks>(&path).unwrap();
        assert_eq!((loaded.key.as_str(), loaded.total), ("liked", 0));
        assert!(age < Duration::from_secs(5));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ignores_other_versions_and_old_files() {
        let dir = dir("versions");
        let path = tracks_path(&dir);
        fs::create_dir_all(&dir).unwrap();
        let data = r#"{"key":"liked","total":0,"tracks":[]}"#;
        fs::write(&path, format!(r#"{{"version":{},"saved_at":{},"data":{data}}}"#, VERSION + 1, now())).unwrap();
        assert!(load::<Tracks>(&path).is_none());
        fs::write(&path, format!(r#"{{"version":{VERSION},"saved_at":0,"data":{data}}}"#)).unwrap();
        assert!(load::<Tracks>(&path).is_none());
        let _ = fs::remove_dir_all(dir);
    }
}
