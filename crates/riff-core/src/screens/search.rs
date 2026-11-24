use std::sync::Arc;

use riff_api::models::{SearchResults, SearchType};
use riff_api::{ApiService, DomainError, Load};

use super::{Fetch, LikedTracks, PageFields, Screen, Update};
use crate::navigation::Route;
use crate::pagination::{Pagination, PAGE_SIZE};

const COMBINED_RESULTS_LIMIT: usize = 24;

pub struct SearchScreen {
    query: String,
    filter: Option<SearchType>,
    pub results: SearchResults,
    pub scope: SearchResults,
    scope_page: Pagination,
    liked: LikedTracks,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchRequest {
    All {
        query: String,
    },
    Scope {
        query: String,
        kind: SearchType,
        offset: usize,
    },
}

impl SearchScreen {
    pub fn new(liked: LikedTracks) -> Self {
        Self {
            query: String::new(),
            filter: None,
            results: SearchResults::default(),
            scope: SearchResults::default(),
            scope_page: Pagination::default(),
            liked,
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn filter(&self) -> Option<SearchType> {
        self.filter
    }

    pub fn search(&mut self, query: &str, filter: Option<SearchType>) -> Vec<SearchRequest> {
        self.query = query.to_string();
        self.filter = filter;
        if query.trim().is_empty() {
            return vec![];
        }
        match filter {
            None => vec![SearchRequest::All {
                query: self.query.clone(),
            }],
            Some(kind) => {
                self.scope_page = Pagination::default();
                self.scope_request(kind).into_iter().collect()
            }
        }
    }

    pub fn has_more(&self) -> bool {
        self.scope_page.has_more()
    }

    pub fn is_complete(&self) -> bool {
        !self.has_more()
    }

    fn scope_request(&mut self, kind: SearchType) -> Option<SearchRequest> {
        let offset = self.scope_page.next_offset()?;
        Some(SearchRequest::Scope {
            query: self.query.clone(),
            kind,
            offset,
        })
    }

    fn add_scope(&mut self, kind: SearchType, offset: usize, results: SearchResults) {
        if offset == 0 {
            self.scope = SearchResults::default();
        }
        let added = match kind {
            SearchType::Track => append(&mut self.scope.tracks.items, results.tracks.items),
            SearchType::Album => append(&mut self.scope.albums.items, results.albums.items),
            SearchType::Artist => append(&mut self.scope.artists.items, results.artists.items),
            SearchType::Playlist => {
                append(&mut self.scope.playlists.items, results.playlists.items)
            }
        };
        self.scope_page.loaded(offset == 0, added);
    }
}

fn append<T>(items: &mut Vec<T>, more: Vec<T>) -> usize {
    let added = more.len();
    items.extend(more);
    added
}

impl Screen for SearchScreen {
    type Request = SearchRequest;
    type Response = SearchResults;

    fn route(&self) -> Route {
        Route::Search
    }

    fn open(&mut self) -> Vec<SearchRequest> {
        let query = self.query.clone();
        self.search(&query, self.filter)
    }

    fn load_more(&mut self) -> Vec<SearchRequest> {
        match self.filter {
            Some(kind) => self.scope_request(kind).into_iter().collect(),
            None => vec![],
        }
    }

    fn fetch(
        &self,
        api: Arc<ApiService>,
        request: SearchRequest,
        load: Load,
    ) -> Fetch<SearchResults> {
        Box::pin(async move {
            match request {
                SearchRequest::All { query } => {
                    api.search(&query, 0, COMBINED_RESULTS_LIMIT, load).await
                }
                SearchRequest::Scope {
                    query,
                    kind,
                    offset,
                } => {
                    api.search_scoped(&query, kind, offset, PAGE_SIZE, load)
                        .await
                }
            }
        })
    }

    fn receive(
        &mut self,
        request: SearchRequest,
        result: Result<SearchResults, DomainError>,
    ) -> Update<SearchRequest> {
        match request {
            SearchRequest::All { query } => {
                if query != self.query || self.filter.is_some() {
                    return Update::default();
                }
                match result {
                    Ok(results) => {
                        self.liked.learn(&results.tracks.items);
                        self.results = results;
                        Update::changed(PageFields::INFO)
                    }
                    Err(error) => Update::error(error),
                }
            }
            SearchRequest::Scope {
                query,
                kind,
                offset,
            } => {
                if query != self.query || self.filter != Some(kind) {
                    return Update::default();
                }
                let Some(at) = self.scope_page.answered(offset) else {
                    return Update::default();
                };
                match result {
                    Ok(results) => {
                        self.liked.learn(&results.tracks.items);
                        self.add_scope(kind, offset, results);
                        Update::changed(PageFields::CARDS)
                    }
                    Err(error) => {
                        self.scope_page.retry_at(at);
                        Update::error(error)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{page, tracks};

    fn track_results(range: std::ops::Range<usize>) -> SearchResults {
        SearchResults {
            tracks: page(tracks("t", range), 0),
            ..Default::default()
        }
    }

    #[test]
    fn an_empty_query_searches_nothing() {
        let mut screen = SearchScreen::new(LikedTracks::default());
        assert!(screen.search("  ", None).is_empty());
    }

    #[test]
    fn filtered_results_come_page_by_page() {
        let mut screen = SearchScreen::new(LikedTracks::default());
        let requests = screen.search("q", Some(SearchType::Track));
        assert_eq!(
            requests,
            [SearchRequest::Scope {
                query: "q".into(),
                kind: SearchType::Track,
                offset: 0
            }]
        );
        let update = screen.receive(requests[0].clone(), Ok(track_results(0..50)));
        assert_eq!(update.changed, PageFields::CARDS);
        let next = screen.load_more();
        assert!(matches!(
            next[..],
            [SearchRequest::Scope { offset: 50, .. }]
        ));
        assert!(screen.load_more().is_empty(), "one page at a time");
        screen.receive(next[0].clone(), Ok(track_results(50..60)));
        assert_eq!(screen.scope.tracks.items.len(), 60);
        assert!(screen.is_complete());
    }

    #[test]
    fn stale_results_are_dropped() {
        let mut screen = SearchScreen::new(LikedTracks::default());
        let old = screen.search("old", None);
        screen.search("new", None);
        let update = screen.receive(old[0].clone(), Ok(track_results(0..3)));
        assert_eq!(update.changed, PageFields::empty());
        assert!(screen.results.tracks.items.is_empty());

        let scoped = screen.search("new", Some(SearchType::Album));
        screen.search("new", None);
        let update = screen.receive(scoped[0].clone(), Ok(track_results(0..3)));
        assert_eq!(update.changed, PageFields::empty());
        let first = screen.search("again", Some(SearchType::Track));
        screen.receive(first[0].clone(), Ok(track_results(0..50)));
        let earlier = screen.load_more();
        let reopened = screen.open();
        screen.receive(reopened[0].clone(), Ok(track_results(0..50)));
        let update = screen.receive(earlier[0].clone(), Ok(track_results(50..100)));
        assert_eq!(
            update.changed,
            PageFields::empty(),
            "a page from before reopening"
        );
        assert_eq!(screen.scope.tracks.items.len(), 50);
    }
}
