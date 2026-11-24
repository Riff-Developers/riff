#[macro_use]
extern crate log;

pub mod models;
pub mod remote;

mod defaults;
mod error;
mod token;

pub(crate) mod cache;
pub(crate) mod http;
mod image;
pub(crate) mod providers;
mod scheduler;
pub(crate) mod service;

#[cfg(debug_assertions)]
pub(crate) mod dev;

pub use error::DomainError;
pub use providers::spotify::domain::{check_user_profile, UserProfileCheck};
pub use providers::spotify::{parse_link, spotify_service};
pub use remote::ContentUris;
pub use scheduler::{Load, LoadPriority, BACKGROUND_EPOCH};
pub use service::{ApiService, TrackLookup};

#[cfg(debug_assertions)]
pub use dev::{is_simulate_offline, set_injected_error, set_simulate_offline};

pub use models::{Device, DeviceKind, RepeatMode};
pub use token::TokenProvider;
