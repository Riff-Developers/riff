#[macro_use]
extern crate log;

macro_rules! impl_from {
    ($target:ident { $($ty:ty => $variant:ident),* $(,)? }) => {
        $(impl From<$ty> for $target {
            fn from(value: $ty) -> Self {
                Self::$variant(value.into())
            }
        })*
    };
}

pub mod browse;
pub mod events;
pub mod library;
pub mod load;
pub mod navigation;
pub mod pagination;
pub mod play_queue;
pub mod playback;
mod position;
pub mod remote;
pub mod schedule;
pub mod screens;
pub mod selection;
pub mod session;
pub mod state;
#[cfg(test)]
mod test_support;
