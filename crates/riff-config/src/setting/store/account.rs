use super::SettingsStore;
use crate::setting::Setting;

impl SettingsStore {
    pub fn drm_verified_user(&self) -> String {
        self.string(Setting::DrmVerifiedUser)
    }

    pub fn set_drm_verified_user(&self, user_id: &str) {
        if let Err(e) = self.set_string(Setting::DrmVerifiedUser, user_id) {
            error!("Failed to save the DRM-verified user: {e}");
        }
    }

    pub fn soloist_paired(&self) -> bool {
        self.bool(Setting::SoloistPaired)
    }

    pub fn set_soloist_paired(&self, paired: bool) {
        if let Err(e) = self.set_bool(Setting::SoloistPaired, paired) {
            error!("Failed to save whether Soloist is paired: {e}");
        }
    }

    #[cfg(debug_assertions)]
    pub fn clear_drm_verified_user(&self) {
        self.set_drm_verified_user("");
    }
}
