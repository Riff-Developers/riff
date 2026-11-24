use riff_api::models::{
    Album, AlbumType, ContentRating, ImageSet, Page, Playlist, ResourceId, Track,
};

use crate::events::{DeviceRequest, Event, ItemId, SavedChanged};
use crate::remote::Intent;

pub fn requests(events: &[Event]) -> Vec<Intent> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::DeviceRequest(DeviceRequest::Control(intent)) => Some(intent.clone()),
            _ => None,
        })
        .collect()
}

pub fn saved_tracks(ids: &[&str], saved: bool) -> Event {
    Event::Saved(SavedChanged {
        item: ItemId::Tracks(ids.iter().map(|id| id.to_string()).collect()),
        saved,
    })
}

pub fn rri(id: &str) -> ResourceId {
    ResourceId {
        id: id.to_string(),
        ..Default::default()
    }
}

pub fn track(id: &str) -> Track {
    Track {
        rri: rri(id),
        title: id.to_string(),
        artists: vec![],
        album: None,
        duration_ms: 1000,
        track_number: None,
        disc_number: None,
        content_rating: ContentRating::default(),
        isrc: None,
        art: ImageSet::default(),
        playable: true,
        popularity: None,
        saved: None,
        preview_url: None,
        url: None,
    }
}

pub fn tracks(prefix: &str, range: std::ops::Range<usize>) -> Vec<Track> {
    range.map(|i| track(&format!("{prefix}{i}"))).collect()
}

pub fn page<T>(items: Vec<T>, offset: usize) -> Page<T> {
    Page {
        items,
        offset: Some(offset),
        total: None,
        next_cursor: None,
    }
}

pub fn playlist(id: &str) -> Playlist {
    Playlist {
        rri: rri(id),
        title: id.to_string(),
        ..Default::default()
    }
}

pub fn album(id: &str) -> Album {
    Album {
        rri: rri(id),
        title: id.to_string(),
        artists: vec![],
        art: ImageSet::default(),
        album_type: AlbumType::Album,
        release_date: None,
        total_tracks: None,
        label: None,
        copyright: None,
        upc: None,
        genres: vec![],
        popularity: None,
        content_rating: ContentRating::default(),
        saved: None,
        tracks: None,
        url: None,
    }
}
