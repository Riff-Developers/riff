mod actions;

use riff_api::models::SearchType;

use crate::events::{Event, NavigationChanged, SearchChanged, SearchFields};
use crate::load;
use crate::navigation::{Navigator, Route, Transition};

pub use actions::{BrowserAction, LibraryAction, NavigationAction, SearchAction};

#[derive(Default)]
pub struct Browser {
    navigator: Navigator,
    query: String,
    filter: Option<SearchType>,
}

impl Browser {
    pub fn current_screen(&self) -> &Route {
        self.navigator.current_route()
    }

    pub fn navigator(&self) -> &Navigator {
        &self.navigator
    }

    pub fn search_query(&self) -> &str {
        &self.query
    }

    pub fn search_filter(&self) -> Option<SearchType> {
        self.filter
    }

    pub fn record_title(&mut self, route: &Route, title: &str, hidden: bool) -> Vec<Event> {
        if self.navigator.set_title(route, title) {
            vec![NavigationChanged::history(hidden).into()]
        } else {
            vec![]
        }
    }

    pub fn apply(&mut self, action: &BrowserAction, hidden: bool) -> Vec<Event> {
        match action {
            BrowserAction::Navigation(action) => self.navigate(action, hidden),
            BrowserAction::Search(SearchAction::Query(query)) => {
                let transition = self.navigator.navigate(Route::Search);
                let mut events = self.go(transition, hidden);
                if *query != self.query {
                    self.query = query.clone();
                    events.push(
                        SearchChanged {
                            changed: SearchFields::QUERY,
                        }
                        .into(),
                    );
                }
                events
            }
            BrowserAction::Search(SearchAction::Filter(filter)) if *filter != self.filter => {
                self.filter = *filter;
                vec![SearchChanged {
                    changed: SearchFields::FILTER,
                }
                .into()]
            }
            BrowserAction::Search(_) | BrowserAction::Library(_) => vec![],
        }
    }

    fn navigate(&mut self, action: &NavigationAction, hidden: bool) -> Vec<Event> {
        let transition = match action {
            NavigationAction::Go(route) => self.navigator.navigate(route.clone()),
            NavigationAction::Back => self.navigator.back(),
            NavigationAction::Forward => self.navigator.forward(),
            NavigationAction::GoTo(id) => self.navigator.go_to(*id),
            NavigationAction::Leave(route) => self.navigator.leave_if_current(route),
            NavigationAction::SaveScroll(id, scroll) => {
                self.navigator.save_scroll(*id, *scroll);
                None
            }
        };
        self.go(transition, hidden)
    }

    fn go(&mut self, transition: Option<Transition>, hidden: bool) -> Vec<Event> {
        let Some(transition) = transition else {
            return vec![];
        };
        load::bump_epoch();
        vec![NavigationChanged::moved(transition, hidden).into()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::{Direction, HomePage};

    const LIBRARY: Route = Route::Home(HomePage::Library);

    fn artist(id: &str) -> Route {
        Route::Artist(id.to_string())
    }

    fn album(id: &str) -> Route {
        Route::Album(id.to_string())
    }

    fn navigate(browser: &mut Browser, route: Route) -> Vec<Event> {
        browser.apply(&NavigationAction::Go(route).into(), false)
    }

    fn back(browser: &mut Browser) -> Vec<Event> {
        browser.apply(&NavigationAction::Back.into(), false)
    }

    fn forward(browser: &mut Browser) -> Vec<Event> {
        browser.apply(&NavigationAction::Forward.into(), false)
    }

    #[test]
    fn every_navigation_advances_the_epoch() {
        let _epoch = load::epoch_lock();
        let mut browser = Browser::default();

        let before_push = load::current_epoch();
        navigate(&mut browser, artist("some_id"));
        let after_push = load::current_epoch();
        assert!(after_push > before_push, "going somewhere new");

        back(&mut browser);
        let after_back = load::current_epoch();
        assert!(after_back > after_push, "going back");

        forward(&mut browser);
        assert!(load::current_epoch() > after_back, "going forward");
    }

    #[test]
    fn renavigating_to_the_current_screen_is_not_a_navigation() {
        let _epoch = load::epoch_lock();
        let mut browser = Browser::default();
        navigate(&mut browser, artist("current"));

        let before = load::current_epoch();
        assert!(navigate(&mut browser, artist("current")).is_empty());
        assert_eq!(load::current_epoch(), before);
    }

    #[test]
    fn navigating_moves_forward() {
        let _epoch = load::epoch_lock();
        let mut browser = Browser::default();
        assert_eq!(*browser.current_screen(), LIBRARY);

        let events = navigate(&mut browser, artist("some_id"));

        assert_eq!(browser.current_screen(), &artist("some_id"));
        assert!(matches!(
            events.as_slice(),
            [Event::Navigation(NavigationChanged {
                transition: Some(Transition {
                    direction: Direction::Forward,
                    ..
                }),
                ..
            })]
        ));
    }

    #[test]
    fn back_and_forward() {
        let _epoch = load::epoch_lock();
        let mut browser = Browser::default();
        navigate(&mut browser, artist("a"));
        navigate(&mut browser, album("b"));

        back(&mut browser);
        assert_eq!(browser.current_screen(), &artist("a"));
        assert!(browser.navigator().can_go_forward());

        forward(&mut browser);
        assert_eq!(browser.current_screen(), &album("b"));
        assert!(forward(&mut browser).is_empty());
    }

    #[test]
    fn back_at_the_start_does_nothing() {
        let _epoch = load::epoch_lock();
        let mut browser = Browser::default();
        assert_eq!(back(&mut browser), vec![]);
        assert_eq!(*browser.current_screen(), LIBRARY);
    }

    #[test]
    fn searching_opens_the_search_screen() {
        let _epoch = load::epoch_lock();
        let mut browser = Browser::default();
        let events = browser.apply(&SearchAction::Query("q".to_string()).into(), false);
        assert_eq!(browser.current_screen(), &Route::Search);
        assert_eq!(browser.search_query(), "q");
        assert_eq!(events.len(), 2);
        assert!(browser.navigator().can_go_back());

        let events = browser.apply(&SearchAction::Filter(Some(SearchType::Track)).into(), false);
        assert_eq!(events.len(), 1);
        assert_eq!(browser.search_filter(), Some(SearchType::Track));
    }

    #[test]
    fn leaving_a_missing_item_only_when_current() {
        let _epoch = load::epoch_lock();
        let mut browser = Browser::default();
        navigate(&mut browser, album("missing"));
        navigate(&mut browser, artist("b"));

        let events = browser.apply(&NavigationAction::Leave(album("missing")).into(), false);
        assert!(events.is_empty());
        assert_eq!(browser.current_screen(), &artist("b"));
    }
}
