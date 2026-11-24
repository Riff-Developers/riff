use crate::events::ItemId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PinKind {
    Playlist,
    Album,
    Artist,
    Track,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub id: String,
    pub kind: PinKind,
    pub title: Option<String>,
}

impl Pin {
    pub fn new(kind: PinKind, id: impl Into<String>, title: Option<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            title,
        }
    }

    fn is(&self, kind: PinKind, id: &str) -> bool {
        self.kind == kind && self.id == id
    }
}

pub(super) fn pins_of(item: &ItemId) -> Vec<(PinKind, &str)> {
    match item {
        ItemId::Album(id) => vec![(PinKind::Album, id)],
        ItemId::Artist(id) => vec![(PinKind::Artist, id)],
        ItemId::Playlist(id) => vec![(PinKind::Playlist, id)],
        ItemId::Tracks(ids) => ids.iter().map(|id| (PinKind::Track, id.as_str())).collect(),
    }
}

#[derive(Debug, Default)]
pub struct Pins {
    pins: Vec<Pin>,
    loaded: bool,
}

impl Pins {
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn iter(&self) -> impl Iterator<Item = &Pin> {
        self.pins.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
    }

    pub fn contains(&self, kind: PinKind, id: &str) -> bool {
        self.pins.iter().any(|pin| pin.is(kind, id))
    }

    pub fn ids(&self, kind: PinKind) -> impl Iterator<Item = &str> {
        self.pins
            .iter()
            .filter(move |pin| pin.kind == kind)
            .map(|pin| pin.id.as_str())
    }

    pub(super) fn set(&mut self, pins: Vec<Pin>) -> bool {
        let changed = !self.loaded || self.pins != pins;
        self.pins = pins;
        self.loaded = true;
        changed
    }

    pub(super) fn add(&mut self, pin: Pin) -> bool {
        if self.contains(pin.kind, &pin.id) {
            return false;
        }
        self.pins.push(pin);
        true
    }

    pub(super) fn remove(&mut self, kind: PinKind, id: &str) -> bool {
        let before = self.pins.len();
        self.pins.retain(|pin| !pin.is(kind, id));
        self.pins.len() != before
    }

    pub(super) fn retain_kind(&mut self, kind: PinKind, keep: impl Fn(&str) -> bool) -> bool {
        let before = self.pins.len();
        self.pins.retain(|pin| pin.kind != kind || keep(&pin.id));
        self.pins.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(kind: PinKind, id: &str) -> Pin {
        Pin::new(kind, id, None)
    }

    #[test]
    fn pinning_twice_keeps_one() {
        let mut pins = Pins::default();
        assert!(pins.add(pin(PinKind::Playlist, "p")));
        assert!(!pins.add(pin(PinKind::Playlist, "p")));
        assert_eq!(pins.iter().count(), 1);
    }

    #[test]
    fn kind_is_part_of_identity() {
        let mut pins = Pins::default();
        pins.add(pin(PinKind::Album, "id"));
        pins.add(pin(PinKind::Artist, "id"));
        assert_eq!(pins.iter().count(), 2);
        assert!(pins.remove(PinKind::Album, "id"));
        assert!(pins.contains(PinKind::Artist, "id"));
        assert!(!pins.remove(PinKind::Album, "id"));
    }

    #[test]
    fn pruning_is_by_kind() {
        let mut pins = Pins::default();
        pins.add(pin(PinKind::Playlist, "p1"));
        pins.add(pin(PinKind::Playlist, "p2"));
        pins.add(pin(PinKind::Album, "p2"));
        assert!(pins.retain_kind(PinKind::Playlist, |id| id == "p1"));
        assert_eq!(pins.ids(PinKind::Playlist).collect::<Vec<_>>(), ["p1"]);
        assert!(pins.contains(PinKind::Album, "p2"));
    }

    #[test]
    fn unsaving_tracks_unpins_each() {
        let item = ItemId::Tracks(vec!["a".into(), "b".into()]);
        assert_eq!(
            pins_of(&item),
            [(PinKind::Track, "a"), (PinKind::Track, "b")]
        );
    }
}
