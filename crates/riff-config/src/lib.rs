//! User and app configuration for the Riff workspace.
//!
//! This crate is the single source of truth for configuration, settings,
//! and feature flags.

#[cfg(feature = "gio")]
#[macro_use]
extern crate log;

pub mod api;
pub mod feature_flag;
pub mod setting;

#[cfg(feature = "gio")]
pub use setting::{SetResult, SettingsStore};
pub use setting::{Setting, SettingSet, SettingsChanged, SettingsSupport};
