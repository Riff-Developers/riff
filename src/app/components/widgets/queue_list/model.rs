use gettextrs::gettext;
use gio::prelude::*;
use gio::SimpleActionGroup;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::rc::Rc;

use crate::app::components::{
    build_song_menu, labels, DetailsPageModel, GroupButton, QueueMenuEntry, SongActions,
    TrackListModel,
};
use crate::app::models::{QueueRole, SongListModel, SongModel, Track};
use crate::app::state::Action;
use crate::app::state::{
    DeviceFields, EntryKey, PlaybackAction, PlaybackFields, PlaybackState, SelectionAction,
    SelectionContext, SelectionState,
};
use crate::app::{AppAction, AppEvent, AppModel, Dispatcher};
use riff_core::pagination::PAGE_SIZE;
use riff_core::play_queue::QueueSection;

pub struct QueueListModel {
    base: DetailsPageModel,
    show_now_playing: bool,
    view: SongListModel,
    models: RefCell<HashMap<EntryKey, SongModel>>,
    dragged: RefCell<Option<String>>,
}

fn section_label(section: &QueueSection) -> Option<String> {
    match section {
        QueueSection::NowPlaying => Some(labels::QUEUE_NOW_PLAYING.clone()),
        QueueSection::Queued => Some(labels::QUEUE_NEXT_IN_QUEUE.clone()),
        QueueSection::Context(Some(name)) => Some(labels::queue_context_label(name)),
        QueueSection::Context(None) => Some(labels::QUEUE_NEXT_UP.clone()),
    }
}

impl QueueListModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher, show_now_playing: bool) -> Self {
        let model = Self {
            base: DetailsPageModel::new_without_id(app_model, dispatcher),
            show_now_playing,
            view: SongListModel::new(PAGE_SIZE as u32),
            models: RefCell::new(HashMap::new()),
            dragged: RefCell::new(None),
        };
        model.sync_rows();
        model
    }

    fn sync_rows(&self) {
        let queue = self.queue();
        let tracks: Vec<Track> = queue
            .queue_rows()
            .iter()
            .map(|row| row.track.clone())
            .collect();
        self.base.liked.learn(&tracks);
        let mut models = self.models.borrow_mut();
        let songs: Vec<SongModel> = queue
            .queue_rows()
            .iter()
            .map(|row| {
                let model = match models.get(&row.key) {
                    Some(model)
                        if model.get_id() == row.track.rri.id
                            && model.queue_role() == Some(row.role) =>
                    {
                        model.clone()
                    }
                    _ => {
                        let model = SongModel::new_keyed(row.track.clone(), row.key.to_string());
                        model.set_queue_role(Some(row.role));
                        models.insert(row.key, model.clone());
                        model
                    }
                };
                model.set_group(section_label(&row.section));
                model
            })
            .collect();
        let listed: HashSet<EntryKey> = queue.queue_rows().iter().map(|row| row.key).collect();
        models.retain(|key, _| listed.contains(key));
        drop(queue);
        self.view.clone().replace_models(songs).commit();
    }

    fn queue(&self) -> impl Deref<Target = PlaybackState> + '_ {
        self.base.app_model.map_state(|s| &s.playback)
    }

    pub fn has_queue(&self) -> bool {
        !self.queue().queue_is_empty()
    }

    pub fn selection_context(&self) -> SelectionContext {
        SelectionContext::Queue
    }

    fn listed_keys(&self) -> Vec<EntryKey> {
        let queue = self.queue();
        if self.show_now_playing {
            queue.view_keys().to_vec()
        } else {
            queue.upcoming_keys().to_vec()
        }
    }

    pub fn selectable_keys(&self) -> Vec<String> {
        let queue = self.queue();
        self.listed_keys()
            .into_iter()
            .filter(|&key| queue.view_track(key).is_some())
            .map(|key| key.to_string())
            .collect()
    }

    pub fn is_selecting(&self) -> bool {
        self.base
            .app_model
            .get_state()
            .selection
            .is_selecting_in(true)
    }

    pub fn selected_count(&self) -> usize {
        self.base.app_model.get_state().selection.count()
    }

    pub fn all_selected(&self) -> bool {
        let keys = self.selectable_keys();
        let state = self.base.app_model.get_state();
        !keys.is_empty() && keys.iter().all(|key| state.selection.is_song_selected(key))
    }

    pub fn toggle_select_all(&self) {
        if self.all_selected() {
            self.base
                .dispatcher
                .dispatch(SelectionAction::DeselectAll.into());
        } else {
            self.select_all();
        }
    }

    pub fn cancel_selection(&self) {
        self.base
            .dispatcher
            .dispatch(AppAction::Core(Action::CancelSelection));
    }

    pub fn select_all(&self) {
        let rows: Vec<(String, Track)> = {
            let queue = self.queue();
            self.listed_keys()
                .into_iter()
                .filter_map(|key| Some((key.to_string(), queue.view_track(key)?)))
                .collect()
        };
        self.base
            .dispatcher
            .dispatch(SelectionAction::SelectKeyed(rows).into());
    }

    pub fn play_id(&self, id: &str) {
        let key = self.listed_keys().into_iter().find(|&key| {
            self.queue()
                .view_track(key)
                .is_some_and(|track| track.rri.id == id)
        });
        if let Some(key) = key {
            self.base
                .dispatcher
                .dispatch(PlaybackAction::PlayEntry(key).into());
        }
    }

    fn drop_keys(key: &str, target: &str) -> Option<(EntryKey, EntryKey)> {
        Some((key.parse().ok()?, target.parse().ok()?))
    }

    pub fn toggle_play(&self) {
        self.base.toggle_playback(true, || None);
    }
}

fn queue_entry(row: &SongModel) -> Option<(EntryKey, QueueRole)> {
    Some((row.row_key().parse().ok()?, row.queue_role()?))
}

impl TrackListModel for QueueListModel {
    fn apply_event(&self, event: &AppEvent) {
        let moved = event.queue_changed()
            || event.playback_changed(PlaybackFields::TRACK | PlaybackFields::CONTEXT)
            || event
                .devices()
                .is_some_and(|d| d.changed.contains(DeviceFields::CURRENT));
        if moved {
            self.sync_rows();
        }
    }

    fn update_liked(&self, event: &AppEvent) {
        if let Some(event) = event.core() {
            self.base.liked.apply(event);
        }
    }

    fn song_list_model(&self) -> SongListModel {
        self.view.clone()
    }

    fn is_paused(&self) -> bool {
        self.base.is_paused()
    }

    fn current_song_id(&self) -> Option<String> {
        self.queue().current_song_id()
    }

    fn hides_row(&self, row: &SongModel) -> bool {
        !self.show_now_playing && row.group().as_deref() == Some(labels::QUEUE_NOW_PLAYING.as_str())
    }

    fn is_row_playing(&self, key: &str) -> bool {
        self.queue()
            .current_key()
            .is_some_and(|k| k.to_string() == key)
    }

    fn autoscroll_to_playing(&self) -> bool {
        false
    }

    fn show_album_column(&self) -> bool {
        true
    }

    fn show_loading_skeleton(&self) -> bool {
        false
    }

    fn deselect_song(&self, key: &str) {
        self.base.deselect_song(key);
    }

    fn selection(&self) -> Option<Box<dyn Deref<Target = SelectionState> + '_>> {
        self.base.selection_in(true)
    }

    fn load_more(&self) {}

    fn play_song_at(&self, pos: usize, _id: &str) {
        let key = self.queue().view_key(pos);
        if let Some(key) = key {
            self.base
                .dispatcher
                .dispatch(PlaybackAction::PlayEntry(key).into());
        }
    }

    fn select_song(&self, key: &str) {
        self.select_songs(&[key.to_string()]);
    }

    fn select_songs(&self, keys: &[String]) {
        let rows: Vec<(String, Track)> = {
            let queue = self.queue();
            keys.iter()
                .filter_map(|key| {
                    let track = queue.view_track(key.parse::<EntryKey>().ok()?)?;
                    Some((key.clone(), track))
                })
                .collect()
        };
        if !rows.is_empty() {
            self.base
                .dispatcher
                .dispatch(SelectionAction::SelectKeyed(rows).into());
        }
    }

    fn enable_selection(&self) -> bool {
        self.base
            .enable_selection_with_context(self.selection_context())
    }

    fn is_song_liked(&self, id: &str) -> bool {
        self.base.is_song_liked(id)
    }

    fn toggle_song_like(&self, id: &str) {
        let songs = self.song_list_model();
        self.base.toggle_song_like(&songs, id);
    }

    fn pinned_song_ids(&self) -> Option<HashSet<String>> {
        self.base.pinned_song_ids()
    }

    fn toggle_song_pin(&self, song: &Track) {
        self.base.toggle_song_pin(song);
    }

    fn skip_explicit(&self) -> bool {
        self.base.skip_explicit()
    }

    fn group_button(&self, group: &str) -> Option<GroupButton> {
        (group == labels::QUEUE_NEXT_IN_QUEUE.as_str()).then(|| GroupButton {
            icon_name: "user-trash-symbolic",
            tooltip: labels::CLEAR_QUEUE.clone(),
            destructive: true,
        })
    }

    fn group_button_clicked(&self, group: &str, source: &gtk::Widget) {
        if group != labels::QUEUE_NEXT_IN_QUEUE.as_str() {
            return;
        }
        let dialog = libadwaita::AlertDialog::new(
            // translators: Title of the dialog asking to confirm clearing the play queue, from its trash button.
            Some(&gettext("Clear the Queue?")),
            // translators: Body of the dialog asking to confirm clearing the play queue.
            Some(&gettext("All tracks added to the queue will be removed.")),
        );
        // translators: Button of the dialog asking to confirm clearing the play queue: keeps the queue.
        dialog.add_response("cancel", &gettext("Cancel"));
        // translators: Button of the dialog asking to confirm clearing the play queue: clears it.
        dialog.add_response("clear", &gettext("Clear"));
        dialog.set_response_appearance("clear", libadwaita::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let dispatcher = self.base.dispatcher.clone();
        dialog.choose(Some(source), gio::Cancellable::NONE, move |response| {
            if response.as_str() == "clear" {
                dispatcher.dispatch(PlaybackAction::ClearQueued.into());
            }
        });
    }

    fn is_reorderable(&self) -> bool {
        true
    }

    fn can_drag_row(&self, key: &str) -> bool {
        let Ok(key) = key.parse::<EntryKey>() else {
            return false;
        };
        self.queue().is_movable(key) && !self.is_selection_enabled()
    }

    fn can_drop_row(&self, key: &str, target: &str, after: bool) -> bool {
        Self::drop_keys(key, target)
            .is_some_and(|(key, target)| self.queue().can_move_entry(key, target, after))
    }

    fn drag_started(&self, key: &str) {
        let track = key
            .parse()
            .ok()
            .and_then(|key| self.queue().view_track(key));
        self.dragged.replace(track.map(|track| track.rri.id));
    }

    fn drop_row(&self, key: &str, target: &str, after: bool) {
        let Some(track_id) = self.dragged.take() else {
            return;
        };
        if let Some((key, target)) = Self::drop_keys(key, target) {
            let action = PlaybackAction::MoveEntry {
                key,
                track_id,
                target,
                after,
            };
            self.base.dispatcher.dispatch(action.into());
        }
    }

    fn actions_for(&self, row: &SongModel, song: &Track) -> Option<SimpleActionGroup> {
        let dispatcher = &self.base.dispatcher;
        let group = SimpleActionGroup::new();
        for a in song.make_artist_actions(dispatcher.clone()) {
            group.add_action(&a);
        }
        group.add_action(&song.make_album_action(dispatcher.clone()));
        group.add_action(&song.make_link_action());
        let actions = match queue_entry(row) {
            Some((key, QueueRole::Queued)) => {
                song.make_queue_entry_actions(key, dispatcher.clone())
            }
            Some((key, QueueRole::Context)) => {
                let mut actions = song.make_queue_actions(dispatcher.clone());
                actions.extend(song.make_queue_entry_actions(key, dispatcher.clone()));
                actions
            }
            Some((_, QueueRole::Current)) => song.make_queue_actions(dispatcher.clone()),
            _ => vec![],
        };
        for a in actions {
            group.add_action(&a);
        }
        Some(group)
    }

    fn playlist_picker(&self, song: &Track) -> Option<gtk::Widget> {
        self.base.playlist_picker(song)
    }

    fn menu_for(
        &self,
        row: &SongModel,
        song: &Track,
        liked: bool,
        pinned: Option<bool>,
    ) -> Option<gio::MenuModel> {
        let queue_entry = match queue_entry(row) {
            Some((_, QueueRole::Queued)) => QueueMenuEntry::Queued,
            Some((_, QueueRole::Context)) => QueueMenuEntry::Context,
            Some((_, QueueRole::Current)) => QueueMenuEntry::Add,
            _ => QueueMenuEntry::None,
        };
        Some(build_song_menu(
            song,
            true,
            None,
            queue_entry,
            Some(liked),
            pinned,
            false,
        ))
    }
}

impl crate::app::ProvidesApi for QueueListModel {
    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.base.app_model.api()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::make_track;
    use crate::app::state::{AppState, SelectionFields};
    use crate::app::AppEvent;
    use crate::app::SongsSource;
    use riff_core::playback::test_playing_from;
    use riff_core::remote::PlaybackView;
    use std::cell::Cell;
    use std::sync::Arc;

    fn test_api_service() -> Arc<riff_api::ApiService> {
        let token_provider: Arc<dyn riff_api::TokenProvider> = Arc::new(|| None::<String>);
        Arc::new(riff_api::spotify_service(
            Arc::clone(&token_provider),
            token_provider,
            1024 * 1024,
            1024 * 1024,
        ))
    }

    fn make_model(show_now_playing: bool) -> (Rc<AppModel>, Rc<QueueListModel>) {
        let (sender, _receiver) = futures::channel::mpsc::unbounded();
        std::mem::forget(_receiver);
        let app_model = Rc::new(AppModel::new(AppState::default(), test_api_service()));
        let model = Rc::new(QueueListModel::new(
            app_model.clone(),
            Dispatcher::new(sender),
            show_now_playing,
        ));
        (app_model, model)
    }

    #[test]
    fn test_binding_rows_during_state_updates_doesnt_read_state() {
        let (app_model, model) = make_model(true);
        let bound = Rc::new(Cell::new(0));
        let view = model.song_list_model();
        {
            let model = model.clone();
            let bound = bound.clone();
            view.connect_items_changed(move |view, position, _, added| {
                for i in position..position + added {
                    let row = view.index_continuous(i as usize).unwrap();
                    let track = row.into_description();
                    let _ = row.row_key();
                    model.actions_for(&row, &track);
                    model.menu_for(&row, &track, false, None);
                    if let Some(group) = row.group() {
                        model.group_button(&group);
                    }
                    bound.set(bound.get() + 1);
                }
            });
        }

        let apply = |action: AppAction| {
            for event in app_model.update_state(action) {
                model.apply_event(&event);
            }
        };
        let album = || Some(SongsSource::Album("a".into()));
        let reports = [
            playing(album(), "1", &["x", "x", "y"], &["2", "3", "4"]),
            playing(album(), "2", &[], &["3", "4"]),
            playing(album(), "3", &["z"], &["4"]),
            PlaybackView::default(),
        ];
        for report in reports {
            apply(PlaybackAction::Show(Box::new(report)).into());
        }

        assert!(bound.get() > 0);
    }

    fn tracks(ids: &[&str]) -> Vec<Track> {
        ids.iter().map(|id| make_track(id)).collect()
    }

    fn playing(
        source: Option<SongsSource>,
        current: &str,
        queued: &[&str],
        next: &[&str],
    ) -> PlaybackView {
        let report = test_playing_from(source, make_track(current), &tracks(queued), &tracks(next));
        match report {
            PlaybackAction::Show(view) => *view,
            _ => unreachable!(),
        }
    }

    fn play(app_model: &AppModel, current: &str, queued: &[&str], next: &[&str]) {
        let view = playing(Some(SongsSource::Album("a".into())), current, queued, next);
        app_model.update_state(PlaybackAction::Show(Box::new(view)).into());
    }

    fn row_keys(app_model: &AppModel) -> Vec<String> {
        let state = app_model.get_state();
        state
            .playback
            .view_keys()
            .iter()
            .map(|k| k.to_string())
            .collect()
    }

    #[test]
    fn test_rows_menus_and_drops() {
        let (app_model, model) = make_model(true);
        let album = SongsSource::Album("a".into());
        app_model.update_state(PlaybackAction::SetContextName(album, "An Album".into()).into());
        play(&app_model, "1", &["p", "a", "b", "c"], &["2"]);

        model.sync_rows();
        let view = model.song_list_model();
        let row = |i| view.index_continuous(i).unwrap();
        let (current, queued, context) = (row(0), row(1), row(5));
        assert!(model.is_row_playing(&current.row_key()));
        let has_action = |row: &SongModel, name: &str| {
            model
                .actions_for(row, &row.into_description())
                .is_some_and(|g| g.lookup_action(name).is_some())
        };
        assert!(has_action(&queued, "dequeue") && !has_action(&queued, "queue"));
        assert!(has_action(&current, "queue") && !has_action(&current, "dequeue"));
        assert!(has_action(&context, "queue") && has_action(&context, "dequeue"));
        let button = model.group_button(&queued.group().unwrap()).unwrap();
        assert_eq!(button.tooltip, *labels::CLEAR_QUEUE);
        assert!(button.destructive);
        for other in [&current, &context] {
            assert_eq!(model.group_button(&other.group().unwrap()), None);
        }

        let rows = row_keys(&app_model);
        let [current, p, a, b, c, context] = &rows[..] else {
            panic!("unexpected rows {:?}", rows);
        };
        assert!(model.can_drag_row(c));
        assert!(!model.can_drag_row(current) && model.can_drag_row(context));
        assert!(model.can_drop_row(c, p, false));
        assert!(model.can_drop_row(context, a, true));
        assert!(!model.can_drop_row(b, a, true));
        assert!(!model.can_drop_row(context, context, false));
        assert!(!model.can_drop_row(current, a, true));
        assert!(!model.can_drop_row("abc", a, true));
    }

    #[test]
    fn test_drops_tell_if_the_current_track_is_listed() {
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        let app_model = Rc::new(AppModel::new(AppState::default(), test_api_service()));
        play(&app_model, "1", &[], &["2", "3"]);
        let rows = row_keys(&app_model);

        for show_now_playing in [true, false] {
            let model = QueueListModel::new(
                app_model.clone(),
                Dispatcher::new(sender.clone()),
                show_now_playing,
            );
            assert!(model.can_drop_row(&rows[2], &rows[1], false));
            model.drag_started(&rows[2]);
            model.drop_row(&rows[2], &rows[1], false);
            let action = receiver.try_next().ok().flatten().expect("an action");
            let AppAction::Core(Action::Playback(PlaybackAction::MoveEntry {
                key,
                target,
                after: false,
                ..
            })) = action
            else {
                panic!("not MoveEntry: {:?}", action);
            };
            assert_eq!(
                (key.to_string(), target.to_string()),
                (rows[2].clone(), rows[1].clone())
            );
        }
    }

    #[test]
    fn test_selection_stays_out_of_the_other_lists() {
        let (app_model, model) = make_model(true);
        let page =
            DetailsPageModel::new_without_id(app_model.clone(), model.base.dispatcher.clone());
        play(&app_model, "1", &[], &["2"]);
        let enabled = |selecting: bool| {
            (
                selecting,
                model.is_selection_enabled(),
                page.selection().is_some_and(|s| s.is_selection_enabled()),
            )
        };
        assert_eq!(enabled(model.is_selecting()), (false, false, false));

        app_model.update_state(AppAction::Core(Action::EnableSelection(
            model.selection_context(),
        )));
        assert_eq!(enabled(model.is_selecting()), (true, true, false));
        assert!(!model.all_selected());
        let rows = row_keys(&app_model)
            .into_iter()
            .map(|key| (key, make_track("x")))
            .collect();
        app_model.update_state(SelectionAction::SelectKeyed(rows).into());
        assert!(model.all_selected());
        assert_eq!(model.selected_count(), 2);

        let events = app_model.update_state(AppAction::Core(Action::EnableSelection(
            SelectionContext::Default,
        )));
        assert!(
            matches!(&events[..], [AppEvent::Core(crate::app::state::Event::Selection(selection))]
            if selection.changed == SelectionFields::MODE | SelectionFields::ITEMS)
        );
        assert_eq!(enabled(model.is_selecting()), (false, false, true));
        assert_eq!(model.selected_count(), 0);
        assert!(!model.all_selected());
    }

    #[test]
    fn test_lists_the_current_track_only_with_now_playing() {
        let (sender, mut receiver) = futures::channel::mpsc::unbounded();
        let app_model = Rc::new(AppModel::new(AppState::default(), test_api_service()));
        play(&app_model, "1", &[], &["2", "3"]);
        let probe = QueueListModel::new(app_model.clone(), Dispatcher::new(sender.clone()), true);
        let view = probe.song_list_model();
        let current = view.index_continuous(0).unwrap();

        for show_now_playing in [true, false] {
            let model = QueueListModel::new(
                app_model.clone(),
                Dispatcher::new(sender.clone()),
                show_now_playing,
            );
            assert_eq!(model.hides_row(&current), !show_now_playing);
            assert!(!model.hides_row(&view.index_continuous(1).unwrap()));
            model.play_id("2");
            let action = receiver.try_next().ok().flatten().expect("an action");
            let AppAction::Core(Action::Playback(PlaybackAction::PlayEntry(key))) = action else {
                panic!("not PlayEntry: {:?}", action);
            };
            assert_eq!(
                app_model
                    .get_state()
                    .playback
                    .view_track(key)
                    .unwrap()
                    .rri
                    .id,
                "2"
            );
        }
    }
}
