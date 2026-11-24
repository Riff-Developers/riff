use crate::app::components::SortOrder;
use crate::app::AppEvent;
use crate::app::ProvidesApi;

use super::CardListModel;

pub trait CardListPageModel: CardListModel + ProvidesApi {
    fn page_id(&self) -> &str;

    fn empty_title(&self) -> String;
    fn empty_description(&self) -> String;
    fn empty_icon(&self) -> &str {
        "audio-x-generic-symbolic"
    }

    fn available_sort_orders(&self) -> &[SortOrder] {
        &[
            SortOrder::RecentlyAdded,
            SortOrder::Alphabetic,
            SortOrder::Creator,
        ]
    }

    /// Applies an event to cards the page keeps itself. Runs before
    /// `should_refresh`.
    fn apply_event(&self, _event: &AppEvent) {}

    /// Returns true if this event means the card list data was updated.
    fn should_refresh(&self, event: &AppEvent) -> bool;
}
