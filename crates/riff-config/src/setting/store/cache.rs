use super::SettingsStore;
use crate::setting::Setting;

pub fn megabytes(mb: u32) -> usize {
    mb.max(1) as usize * 1024 * 1024
}

impl SettingsStore {
    pub fn audio_cache_size_limit_bytes(&self) -> u64 {
        megabytes(self.uint(Setting::AudioCacheSizeMb)) as u64
    }
}
