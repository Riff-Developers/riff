use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum SpotifyError {
    LoginFailed,
    NotPremium,
    LoggedOut,
    TechnicalError,
    PlaybackDrmBlocked,
    PlaybackTemporarilyUnavailable,
}

impl Error for SpotifyError {}

impl fmt::Display for SpotifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LoginFailed => write!(f, "Login failed!"),
            Self::NotPremium => write!(f, "A Spotify Premium subscription is required."),
            Self::LoggedOut => write!(f, "You are logged out!"),
            Self::TechnicalError => {
                write!(f, "A technical error occured. Check your connectivity.")
            }
            Self::PlaybackDrmBlocked => write!(
                f,
                "Playback is not available for this account. Spotify's PlayPlay DRM \
                 is proprietary and cannot be implemented outside the official client."
            ),
            Self::PlaybackTemporarilyUnavailable => {
                write!(f, "Playback is temporarily unavailable.")
            }
        }
    }
}
