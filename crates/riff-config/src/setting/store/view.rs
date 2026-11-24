use super::SettingsStore;
use crate::setting::Setting;

impl SettingsStore {
    pub fn card_layout(&self) -> String {
        self.string(Setting::CardLayout)
    }

    pub fn set_card_layout(&self, layout: &str) {
        if let Err(e) = self.set_string(Setting::CardLayout, layout) {
            error!("Failed to save the card layout: {e}");
        }
    }

    pub fn card_size(&self) -> String {
        self.string(Setting::CardSize)
    }

    pub fn set_card_size(&self, size: &str) {
        if let Err(e) = self.set_string(Setting::CardSize, size) {
            error!("Failed to save the card size: {e}");
        }
    }

    pub fn sort_order(&self, page: &str) -> Option<String> {
        Some(self.string(Setting::sort_order(page)?))
    }

    pub fn set_sort_order(&self, page: &str, order: &str) {
        let Some(setting) = Setting::sort_order(page) else {
            return;
        };
        if let Err(e) = self.set_string(setting, order) {
            error!("Failed to save the {page} sort order: {e}");
        }
    }
}
