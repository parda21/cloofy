//! The music services cloofy plays from. Every URI names its service up front, so a request
//! about an item goes to the service the item came from.

/// A music service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    Spotify,
    YouTubeMusic,
}

impl Service {
    pub const ALL: [Service; 2] = [Service::Spotify, Service::YouTubeMusic];

    /// The service a URI such as `spotify:track:…` or `ytmusic:album:…` belongs to.
    pub fn of(uri: &str) -> Option<Service> {
        let (scheme, _) = uri.split_once(':')?;
        Service::from_key(scheme)
    }

    /// The service with this key, as in `spotify` or `ytmusic`.
    pub fn from_key(key: &str) -> Option<Service> {
        Service::ALL.into_iter().find(|service| service.key() == key)
    }

    /// The name people know it by.
    pub fn name(self) -> &'static str {
        match self {
            Service::Spotify => "Spotify",
            Service::YouTubeMusic => "YouTube Music",
        }
    }

    /// Its key in URIs and in the config file.
    pub fn key(self) -> &'static str {
        match self {
            Service::Spotify => "spotify",
            Service::YouTubeMusic => "ytmusic",
        }
    }
}

/// The page of the item a URI names on the service's website, for sharing, and for desktop
/// widgets that tell services apart by it.
pub fn web_link(uri: &str) -> Option<String> {
    let (kind, id) = uri.split_once(':').map(|(_, rest)| rest)?.split_once(':')?;
    if id.is_empty() {
        return None;
    }
    Some(match (Service::of(uri)?, kind) {
        (Service::Spotify, _) => format!("https://open.spotify.com/{kind}/{id}"),
        (Service::YouTubeMusic, "track") => format!("https://music.youtube.com/watch?v={id}"),
        (Service::YouTubeMusic, "playlist") => format!("https://music.youtube.com/playlist?list={id}"),
        (Service::YouTubeMusic, "album") => format!("https://music.youtube.com/browse/{id}"),
        (Service::YouTubeMusic, "artist") => format!("https://music.youtube.com/channel/{id}"),
        (Service::YouTubeMusic, _) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_the_service_by_the_uri() {
        assert_eq!(Service::of("spotify:track:4uLU6hMCjMI75M1A2tKUQC"), Some(Service::Spotify));
        assert_eq!(Service::of("ytmusic:album:MPREb_abc"), Some(Service::YouTubeMusic));
        assert_eq!(Service::of("spotify:"), Some(Service::Spotify));
        assert_eq!(Service::of("https://open.spotify.com/track/1"), None);
        assert_eq!(Service::of("ytmusic"), None);
        assert_eq!(Service::of(""), None);
    }

    #[test]
    fn keys_go_back_to_the_service() {
        for service in Service::ALL {
            assert_eq!(Service::of(&format!("{}:track:1", service.key())), Some(service));
            assert_eq!(Service::from_key(service.key()), Some(service));
        }
        assert_eq!(Service::from_key("youtube"), None);
        assert_eq!(Service::YouTubeMusic.name(), "YouTube Music");
    }

    #[test]
    fn links_to_the_item_on_the_web() {
        assert_eq!(
            web_link("spotify:track:4uLU6hMCjMI75M1A2tKUQC").unwrap(),
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC"
        );
        assert_eq!(
            web_link("spotify:episode:512ojhOuo1ktJprKbVcKyQ").unwrap(),
            "https://open.spotify.com/episode/512ojhOuo1ktJprKbVcKyQ"
        );
        assert_eq!(web_link("ytmusic:track:dQw4w9WgXcQ").unwrap(), "https://music.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(web_link("ytmusic:album:MPREb_abc").unwrap(), "https://music.youtube.com/browse/MPREb_abc");
        assert_eq!(web_link("ytmusic:playlist:PL123").unwrap(), "https://music.youtube.com/playlist?list=PL123");
        assert_eq!(web_link("ytmusic:artist:UC123").unwrap(), "https://music.youtube.com/channel/UC123");
        assert_eq!(web_link("spotify:track:"), None);
        assert_eq!(web_link("spotify:user"), None);
        assert_eq!(web_link("file:track:1"), None);
    }
}
