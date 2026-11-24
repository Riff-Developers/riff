use std::sync::atomic::{AtomicU64, Ordering};

use riff_api::{Load, LoadPriority, BACKGROUND_EPOCH};

static NAV_EPOCH: AtomicU64 = AtomicU64::new(BACKGROUND_EPOCH + 1);

pub fn bump_epoch() -> u64 {
    NAV_EPOCH.fetch_add(1, Ordering::Relaxed) + 1
}

pub fn current_epoch() -> u64 {
    NAV_EPOCH.load(Ordering::Relaxed)
}

pub fn at(priority: LoadPriority) -> Load {
    Load::new(priority, current_epoch())
}

pub fn visible() -> Load {
    at(LoadPriority::Visible)
}

pub fn offscreen() -> Load {
    at(LoadPriority::Offscreen)
}

pub fn hero() -> Load {
    at(LoadPriority::Hero)
}

pub fn transport() -> Load {
    at(LoadPriority::Transport)
}

#[cfg(test)]
pub(crate) fn epoch_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_starts_above_background_and_increases() {
        let _epoch = epoch_lock();
        let first = current_epoch();
        assert!(first > BACKGROUND_EPOCH);
        assert_eq!(bump_epoch(), first + 1);
        assert_eq!(current_epoch(), first + 1);
    }

    #[test]
    fn background_is_below_every_view() {
        assert!(Load::background().epoch < visible().epoch);
        assert_eq!(Load::background().priority, LoadPriority::Background);
    }

    #[test]
    fn transport_outranks_artwork_in_the_same_epoch() {
        let transport = transport();
        let hero = hero();
        assert_eq!(transport.epoch, hero.epoch);
        assert!(transport.priority > hero.priority);
    }

    #[test]
    fn view_priorities_are_ordered() {
        assert!(hero().priority > visible().priority);
        assert!(visible().priority > offscreen().priority);
        assert!(offscreen().priority > Load::background().priority);
    }
}
