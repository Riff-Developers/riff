use riff_api::models::{Page, Track};
use riff_api::DomainError;

use super::{PageFields, Update};
use crate::pagination::Pagination;

#[derive(Debug)]
pub struct Pages<T> {
    pub items: Vec<T>,
    pub total: Option<usize>,
    cursor: Pagination,
}

pub type TrackPages = Pages<Track>;

impl<T> Default for Pages<T> {
    fn default() -> Self {
        Self {
            items: vec![],
            total: None,
            cursor: Pagination::default(),
        }
    }
}

impl<T> Pages<T> {
    pub fn restart(&mut self) -> usize {
        self.cursor.restart()
    }

    pub fn next_offset(&mut self) -> Option<usize> {
        self.cursor.next_offset()
    }

    pub fn next_request<R>(&mut self, request: impl FnOnce(usize) -> R) -> Vec<R> {
        self.next_offset().map(request).into_iter().collect()
    }

    pub fn has_more(&self) -> bool {
        self.cursor.has_more()
    }

    pub fn is_complete(&self) -> bool {
        !self.has_more()
    }

    pub fn failed<R>(&mut self, offset: usize, error: DomainError) -> Update<R> {
        if let Some(at) = self.cursor.answered(offset) {
            self.cursor.retry_at(at);
        }
        Update::error(error)
    }

    pub fn remove(&mut self, matches: impl Fn(&T) -> bool) {
        let before = self.items.len();
        self.items.retain(|item| !matches(item));
        let removed = before - self.items.len();
        self.cursor.shift(0, removed);
        if let Some(total) = self.total.as_mut() {
            *total = total.saturating_sub(removed);
        }
    }
}

impl<T: Clone> Pages<T> {
    pub fn prepend(&mut self, items: &[T]) {
        self.items.splice(0..0, items.iter().cloned());
        self.cursor.shift(items.len(), 0);
        if let Some(total) = self.total.as_mut() {
            *total = total.saturating_add(items.len());
        }
    }
}

impl<T: PartialEq> Pages<T> {
    pub fn loaded<R>(&mut self, offset: usize, page: Page<T>, changed: PageFields) -> Update<R> {
        self.load(offset, page);
        Update::changed(changed)
    }

    pub fn load(&mut self, offset: usize, mut page: Page<T>) {
        let first = offset == 0;
        let at = match self.cursor.answered(offset) {
            Some(at) => at,
            None if first => 0,
            None => return,
        };
        page.offset = Some(at);
        let loaded = page.items.len();
        if let Some(total) = page.total {
            self.total = Some(match self.total {
                Some(known) if !first => known.max(total),
                _ => total,
            });
        }
        if first {
            self.items.clear();
        }
        put_page(&mut self.items, page);
        self.cursor.loaded(first, loaded);
    }
}

fn put_page<T: PartialEq>(list: &mut Vec<T>, page: Page<T>) -> bool {
    let start = page.offset.unwrap_or(0).min(list.len());
    let end = (start + page.items.len()).min(list.len());
    if list[start..end] == page.items[..end - start] && end - start == page.items.len() {
        return false;
    }
    list.splice(start..end, page.items);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{page, tracks};

    #[test]
    fn track_pages_build_up() {
        let mut pages = TrackPages::default();
        assert_eq!(pages.next_offset(), Some(0));
        let mut first = page(tracks("t", 0..50), 0);
        first.total = Some(60);
        pages.load(0, first);
        assert_eq!(pages.next_offset(), Some(50));
        assert_eq!(pages.next_offset(), None, "one page at a time");
        pages.load(50, page(tracks("t", 50..60), 50));
        assert_eq!(pages.items.len(), 60);
        assert!(pages.is_complete());

        pages.prepend(&tracks("new", 0..1));
        assert_eq!(pages.total, Some(61));
        pages.remove(|t| t.rri.id == "t0");
        assert_eq!((pages.items.len(), pages.total), (60, Some(60)));
    }

    #[test]
    fn restarting_waits_for_the_first_page() {
        let mut pages = TrackPages::default();
        pages.load(0, page(tracks("t", 0..50), 0));
        assert_eq!(pages.restart(), 0);
        assert_eq!(pages.next_offset(), None);
        pages.load(0, page(tracks("u", 0..50), 0));
        assert_eq!(pages.items[0].rri.id, "u0");
        assert_eq!(pages.next_offset(), Some(50));
    }

    #[test]
    fn removing_loaded_items_moves_the_next_page_back() {
        let mut pages = TrackPages::default();
        pages.next_offset();
        let mut first = page(tracks("t", 0..50), 0);
        first.total = Some(200);
        pages.load(0, first);
        pages.remove(|t| t.rri.id == "t3");
        assert_eq!(pages.next_offset(), Some(49));
        pages.load(49, page(tracks("t", 50..100), 49));
        assert_eq!(pages.items.len(), 99);
        assert_eq!(pages.items[49].rri.id, "t50");
    }

    #[test]
    fn a_page_asked_for_before_a_removal_lands_where_it_now_goes() {
        let mut pages = TrackPages::default();
        pages.next_offset();
        pages.load(0, page(tracks("t", 0..50), 0));
        assert_eq!(pages.next_offset(), Some(50));
        pages.remove(|t| t.rri.id == "t3");
        pages.failed::<()>(50, DomainError::Shed);
        assert_eq!(
            pages.next_offset(),
            Some(49),
            "it's asked for again where it now starts"
        );

        pages.remove(|t| t.rri.id == "t4");
        pages.load(49, page(tracks("t", 51..101), 49));
        assert_eq!(pages.items.len(), 98);
        assert_eq!(pages.items[48].rri.id, "t51");
    }

    #[test]
    fn a_page_from_before_a_reload_is_left_out() {
        let mut pages = TrackPages::default();
        pages.next_offset();
        pages.load(0, page(tracks("t", 0..50), 0));
        assert_eq!(pages.next_offset(), Some(50));
        pages.restart();
        pages.load(50, page(tracks("old", 50..100), 50));
        assert!(pages.items.iter().all(|t| !t.rri.id.starts_with("old")));
    }

    #[test]
    fn reloading_takes_the_new_total() {
        let mut pages = TrackPages::default();
        let mut first = page(tracks("t", 0..50), 0);
        first.total = Some(200);
        pages.load(0, first);
        pages.restart();
        let mut again = page(tracks("t", 0..50), 0);
        again.total = Some(190);
        pages.load(0, again);
        assert_eq!(pages.total, Some(190));
    }

    #[test]
    fn a_failed_page_is_asked_for_again() {
        let mut pages = TrackPages::default();
        let offset = pages.next_offset().unwrap();
        pages.failed::<()>(offset, DomainError::Shed);
        assert_eq!(pages.next_offset(), Some(0));
    }

    #[test]
    fn pages_land_at_their_offset() {
        let page = |offset: usize, items: Vec<u32>| Page {
            items,
            offset: Some(offset),
            total: None,
            next_cursor: None,
        };
        let mut list = vec![];
        assert!(put_page(&mut list, page(0, vec![1, 2])));
        assert!(put_page(&mut list, page(2, vec![3])));
        assert!(!put_page(&mut list, page(2, vec![3])));
        assert!(put_page(&mut list, page(0, vec![9, 2])));
        assert_eq!(list, [9, 2, 3]);
    }
}
