use crate::models::Link;

pub fn parse_link(input: &str) -> Option<Link> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    if let Some(rest) = input.strip_prefix("spotify:") {
        parse_uri(rest)
    } else {
        parse_url(input)
    }
}

fn parse_uri(rest: &str) -> Option<Link> {
    let rest = rest.trim_start_matches('/');
    let parts: Vec<&str> = rest.split(':').collect();
    from_segments(&parts)
}

fn parse_url(input: &str) -> Option<Link> {
    let rest = input
        .strip_prefix("https://")
        .or_else(|| input.strip_prefix("http://"))?;

    let rest = rest.strip_prefix("open.spotify.com/")?;

    let path = rest
        .split(['?', '#'])
        .next()
        .unwrap_or(rest)
        .trim_matches('/');

    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.first().is_some_and(|s| s.starts_with("intl-")) {
        segments.remove(0);
    }
    from_segments(&segments)
}

fn from_segments(segments: &[&str]) -> Option<Link> {
    match segments {
        ["user", _, "playlist", id, ..] => from_kind_and_id("playlist", id),
        [kind, id, ..] => from_kind_and_id(kind, id),
        _ => None,
    }
}

fn from_kind_and_id(kind: &str, id: &str) -> Option<Link> {
    if id.is_empty() {
        return None;
    }
    let id = id.to_string();
    match kind {
        "album" => Some(Link::Album(id)),
        "artist" => Some(Link::Artist(id)),
        "playlist" => Some(Link::Playlist(id)),
        "user" => Some(Link::User(id)),
        "track" => Some(Link::Track(id)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_uri_album() {
        assert_eq!(
            parse_link("spotify:album:6akEvsycLGftJxYudPjmqK"),
            Some(Link::Album("6akEvsycLGftJxYudPjmqK".to_string()))
        );
    }

    #[test]
    fn test_parse_uri_all_types() {
        assert_eq!(
            parse_link("spotify:artist:abc"),
            Some(Link::Artist("abc".to_string()))
        );
        assert_eq!(
            parse_link("spotify:playlist:abc"),
            Some(Link::Playlist("abc".to_string()))
        );
        assert_eq!(
            parse_link("spotify:user:abc"),
            Some(Link::User("abc".to_string()))
        );
        assert_eq!(
            parse_link("spotify:track:abc"),
            Some(Link::Track("abc".to_string()))
        );
    }

    #[test]
    fn test_parse_uri_with_glib_slashes() {
        assert_eq!(
            parse_link("spotify:///playlist:xyz"),
            Some(Link::Playlist("xyz".to_string()))
        );
    }

    #[test]
    fn test_parse_url_track() {
        assert_eq!(
            parse_link("https://open.spotify.com/track/6rqhFgbbKwnb9MLmUQDhG6"),
            Some(Link::Track("6rqhFgbbKwnb9MLmUQDhG6".to_string()))
        );
    }

    #[test]
    fn test_parse_url_with_query() {
        assert_eq!(
            parse_link("https://open.spotify.com/album/6akEvsycLGftJxYudPjmqK?si=abcdef123456"),
            Some(Link::Album("6akEvsycLGftJxYudPjmqK".to_string()))
        );
    }

    #[test]
    fn test_parse_url_with_locale_prefix() {
        assert_eq!(
            parse_link("https://open.spotify.com/intl-de/track/abc?si=xyz"),
            Some(Link::Track("abc".to_string()))
        );
    }

    #[test]
    fn test_parse_url_http_scheme() {
        assert_eq!(
            parse_link("http://open.spotify.com/artist/abc"),
            Some(Link::Artist("abc".to_string()))
        );
    }

    #[test]
    fn test_parse_url_trailing_slash() {
        assert_eq!(
            parse_link("https://open.spotify.com/playlist/abc/"),
            Some(Link::Playlist("abc".to_string()))
        );
    }

    #[test]
    fn test_parse_legacy_user_playlist() {
        assert_eq!(
            parse_link("spotify:user:spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"),
            Some(Link::Playlist("37i9dQZF1DXcBWIGoYBM5M".to_string()))
        );
        assert_eq!(
            parse_link("https://open.spotify.com/user/spotify/playlist/37i9dQZF1DXcBWIGoYBM5M"),
            Some(Link::Playlist("37i9dQZF1DXcBWIGoYBM5M".to_string()))
        );
        assert_eq!(
            parse_link("https://open.spotify.com/user/spotify"),
            Some(Link::User("spotify".to_string()))
        );
    }

    #[test]
    fn test_parse_with_surrounding_whitespace() {
        assert_eq!(
            parse_link("  spotify:album:abc\n"),
            Some(Link::Album("abc".to_string()))
        );
    }

    #[test]
    fn test_reject_non_spotify() {
        assert_eq!(parse_link(""), None);
        assert_eq!(parse_link("hello world"), None);
        assert_eq!(parse_link("https://example.com/track/abc"), None);
        assert_eq!(parse_link("https://open.spotify.com/unknown/abc"), None);
        assert_eq!(parse_link("spotify:album:"), None);
        assert_eq!(parse_link("spotify:album"), None);
        assert_eq!(parse_link("https://open.spotify.com/track"), None);
    }
}
