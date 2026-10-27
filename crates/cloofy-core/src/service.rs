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
        Service::ALL.into_iter().find(|service| service.key() == scheme)
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
        }
        assert_eq!(Service::YouTubeMusic.name(), "YouTube Music");
    }
}
