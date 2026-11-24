use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct PlaylistFields: u8 {
        const INFO = 1 << 0;
        const TRACKS = 1 << 1;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct LibraryFields: u8 {
        const PLAYLISTS = 1 << 0;
        const PINS = 1 << 1;
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct SearchFields: u8 {
        const QUERY = 1 << 0;
        const FILTER = 1 << 1;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaylistUpdated {
    pub id: String,
    pub changed: PlaylistFields,
}

impl PlaylistUpdated {
    pub fn new(id: impl Into<String>, changed: PlaylistFields) -> Self {
        Self {
            id: id.into(),
            changed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LibraryChanged {
    pub changed: LibraryFields,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchChanged {
    pub changed: SearchFields,
}
