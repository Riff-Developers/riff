use std::future::Future;

use riff_api::models::Page;
use riff_api::DomainError;

pub const PAGE_SIZE: usize = 50;

pub async fn load_all<T, F, Fut>(mut page_at: F) -> Result<Vec<T>, DomainError>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<Page<T>, DomainError>>,
{
    let mut items = vec![];
    loop {
        let page = page_at(items.len()).await?;
        let count = page.items.len();
        items.extend(page.items);
        if count < PAGE_SIZE || page.total.is_some_and(|total| items.len() >= total) {
            return Ok(items);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Asked {
    offset: usize,
    at: usize,
}

#[derive(Clone, Debug)]
pub struct Pagination {
    next_offset: Option<usize>,
    asked: Option<Asked>,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            next_offset: Some(0),
            asked: None,
        }
    }
}

impl Pagination {
    pub fn has_more(&self) -> bool {
        self.next_offset.is_some()
    }

    pub fn restart(&mut self) -> usize {
        self.next_offset = None;
        self.asked = Some(Asked { offset: 0, at: 0 });
        0
    }

    pub fn next_offset(&mut self) -> Option<usize> {
        if self.asked.is_some() {
            return None;
        }
        let offset = self.next_offset.take()?;
        self.next_offset = Some(offset + PAGE_SIZE);
        self.asked = Some(Asked { offset, at: offset });
        Some(offset)
    }

    pub fn answered(&mut self, offset: usize) -> Option<usize> {
        let asked = self.asked.filter(|asked| asked.offset == offset)?;
        self.asked = None;
        Some(asked.at)
    }

    pub fn retry_at(&mut self, at: usize) {
        self.next_offset = Some(at);
    }

    pub fn loaded(&mut self, first: bool, count: usize) {
        if count < PAGE_SIZE {
            self.next_offset = None;
        } else if first {
            self.next_offset = Some(PAGE_SIZE);
        }
    }

    pub fn shift(&mut self, added: usize, removed: usize) {
        let moved = |offset: &mut usize| *offset = (*offset + added).saturating_sub(removed);
        if let Some(offset) = self.next_offset.as_mut() {
            moved(offset);
        }
        if let Some(asked) = self.asked.as_mut() {
            moved(&mut asked.at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_page_is_asked_for_at_a_time() {
        let mut p = Pagination::default();
        assert_eq!(p.next_offset(), Some(0));
        assert_eq!(p.next_offset(), None);
        assert_eq!(p.answered(0), Some(0));
        p.loaded(true, PAGE_SIZE);
        assert_eq!(p.next_offset(), Some(PAGE_SIZE));
    }

    #[test]
    fn a_short_page_is_the_last() {
        let mut p = Pagination::default();
        p.next_offset();
        p.answered(0);
        p.loaded(true, 10);
        assert!(!p.has_more());
    }

    #[test]
    fn a_failed_page_is_asked_for_again() {
        let mut p = Pagination::default();
        p.next_offset();
        p.answered(0);
        p.loaded(true, PAGE_SIZE);
        let second = p.next_offset().unwrap();
        let at = p.answered(second).unwrap();
        p.retry_at(at);
        assert_eq!(p.next_offset(), Some(second));
    }

    #[test]
    fn only_the_asked_page_is_answered() {
        let mut p = Pagination::default();
        p.next_offset();
        assert_eq!(p.answered(PAGE_SIZE), None);
        assert_eq!(p.answered(0), Some(0));
        assert_eq!(p.answered(0), None);
    }
}
