//! Pieces the internal API's responses share: texts, covers and playlists.

use cloofy_core::catalog::PlaylistInfo;
use serde_json::Value;

/// Preferred cover width; Spotify offers roughly 64, 300 and 640 px.
const IMAGE_WIDTH: u64 = 300;

/// A playlist as Home, the library and search describe it. They leave out how many songs it
/// has, and whether it is collaborative.
pub fn playlist(data: &Value) -> Option<PlaylistInfo> {
    if data.get("__typename")?.as_str()? != "Playlist" {
        return None;
    }
    let uri = text(data, "/uri");
    let name = text(data, "/name");
    if uri.is_empty() || name.is_empty() {
        return None;
    }
    Some(PlaylistInfo {
        id: last_part(&uri),
        name,
        owner_id: last_part(&text(data, "/ownerV2/data/uri")),
        owner_name: text(data, "/ownerV2/data/name"),
        collaborative: false,
        total: None,
        image_url: image(data.pointer("/images/items/0/sources")),
        uri,
    })
}

/// The source closest to the preferred width; one without a width counts as a match.
pub fn image(sources: Option<&Value>) -> Option<String> {
    sources?
        .as_array()?
        .iter()
        .min_by_key(|s| s["width"].as_u64().unwrap_or(IMAGE_WIDTH).abs_diff(IMAGE_WIDTH))
        .and_then(|s| s["url"].as_str())
        .map(str::to_owned)
}

pub fn names(list: Option<&Value>, pointer: &str) -> String {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.pointer(pointer).and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn text(value: &Value, pointer: &str) -> String {
    value.pointer(pointer).and_then(Value::as_str).unwrap_or_default().to_owned()
}

/// The ID at the end of a URI like `spotify:user:spotify`.
fn last_part(uri: &str) -> String {
    uri.rsplit(':').next().unwrap_or_default().to_owned()
}

/// Playlist descriptions contain links like `<a href=…>Artist</a>`.
pub fn strip_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&#x27;", "'").replace("&quot;", "\"")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn strips_links_from_descriptions() {
        assert_eq!(strip_html(r#"With <a href="spotify:artist:1">Daft Punk</a> &amp; more"#), "With Daft Punk & more");
    }

    #[test]
    fn reads_a_mix() {
        let data = json!({
            "__typename": "Playlist",
            "uri": "spotify:playlist:37i9dQZF1EIhdusJHpCNoA",
            "name": "Synthwave Mix",
            "format": "descripto",
            "images": { "items": [{ "sources": [
                { "url": "https://seed-mix-image.spotifycdn.com/v6/img/desc/Synthwave/en/default", "width": null }
            ] }] },
            "ownerV2": { "data": { "__typename": "User", "name": "Spotify", "uri": "spotify:user:spotify" } }
        });
        let playlist = playlist(&data).unwrap();
        assert_eq!(playlist.id, "37i9dQZF1EIhdusJHpCNoA");
        assert_eq!((playlist.owner_id.as_str(), playlist.owner_name.as_str()), ("spotify", "Spotify"));
        assert_eq!(playlist.total, None);
        assert!(playlist.image_url.unwrap().contains("Synthwave"));
        assert!(super::playlist(&json!({ "__typename": "NotFound" })).is_none());
    }
}
