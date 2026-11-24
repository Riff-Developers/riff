use bitflags::bitflags;
use url::Url;

bitflags! {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct SessionFields: u8 {
        const STATUS = 1 << 0;
        const CONNECTION = 1 << 1;
        const ENGINE = 1 << 2;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum EngineStatus {
    #[default]
    BuiltIn,
    Installing,
    Updating,
    NeedsKey {
        key_page: String,
        refused: bool,
    },
    NeedsPairing {
        device_name: String,
    },
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginMethod {
    Saved,
    Browser,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SessionStatus {
    #[default]
    LoggedOut,
    LoggingIn(LoginMethod),
    WaitingForBrowser(Url),
    LoggedIn,
    Failed,
    NotPremium,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionChanged {
    pub changed: SessionFields,
    pub status: SessionStatus,
    pub connection_lost: bool,
    pub engine: EngineStatus,
}

impl SessionChanged {
    pub fn became(&self, status: &SessionStatus) -> bool {
        self.changed.contains(SessionFields::STATUS) && &self.status == status
    }
}
