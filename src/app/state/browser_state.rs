use crate::app::components::{CardLayout, CardSize, SortOrder};
use crate::app::state::{AppEvent, Route};
pub use riff_core::screens::PageFields;

#[derive(Eq, PartialEq, Clone, Debug)]
pub enum ViewPreferencesChanged {
    CardLayout(CardLayout),
    CardSize(CardSize),
    SortOrder(String, SortOrder),
}

impl From<ViewPreferencesChanged> for AppEvent {
    fn from(event: ViewPreferencesChanged) -> Self {
        Self::ViewPreferences(event)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageLoaded {
    pub route: Route,
    pub changed: PageFields,
}

impl From<PageLoaded> for AppEvent {
    fn from(event: PageLoaded) -> Self {
        Self::PageLoaded(event)
    }
}
