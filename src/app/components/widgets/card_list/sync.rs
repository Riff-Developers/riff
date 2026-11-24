use gio::prelude::*;

use crate::app::models::CardModel;
use crate::app::ListStore;

fn same_card(a: &CardModel, b: &CardModel) -> bool {
    a.id() == b.id()
        && a.title() == b.title()
        && a.subtitle() == b.subtitle()
        && a.image() == b.image()
}

pub fn sync_cards(store: &ListStore<CardModel>, cards: Vec<CardModel>) {
    let current: Vec<CardModel> = store.iter().collect();
    let prefix = current
        .iter()
        .zip(&cards)
        .take_while(|(a, b)| same_card(a, b))
        .count();
    let suffix = current[prefix..]
        .iter()
        .rev()
        .zip(cards[prefix..].iter().rev())
        .take_while(|(a, b)| same_card(a, b))
        .count();
    let removed = current.len() - prefix - suffix;
    let added: Vec<glib::Object> = cards[prefix..cards.len() - suffix]
        .iter()
        .map(|card| card.clone().upcast())
        .collect();
    if removed > 0 || !added.is_empty() {
        store.inner().splice(prefix as u32, removed as u32, &added);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, title: &str) -> CardModel {
        CardModel::new(id, None, title, "", None, None, None, None)
    }

    fn shown(store: &ListStore<CardModel>) -> Vec<String> {
        store
            .iter()
            .map(|c| format!("{}:{}", c.id(), c.title()))
            .collect()
    }

    #[test]
    fn only_changed_cards_are_replaced() {
        let store = ListStore::new();
        sync_cards(&store, vec![card("a", "A"), card("b", "B"), card("c", "C")]);
        let kept = store.get(2);

        sync_cards(
            &store,
            vec![
                card("new", "N"),
                card("a", "A"),
                card("b", "Bee"),
                card("c", "C"),
            ],
        );
        assert_eq!(shown(&store), ["new:N", "a:A", "b:Bee", "c:C"]);
        assert_eq!(store.get(3), kept);

        sync_cards(&store, vec![card("a", "A"), card("c", "C")]);
        assert_eq!(shown(&store), ["a:A", "c:C"]);
        assert_eq!(store.get(1), kept);
    }
}
