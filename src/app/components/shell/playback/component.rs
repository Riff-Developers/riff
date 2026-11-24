use std::ops::Deref;
use std::rc::Rc;

use crate::app::components::EventListener;
use crate::app::models::*;
use crate::app::state::NavigationAction;
use crate::app::state::{Event, HomePage, PlaybackAction, PlaybackFields, Route, SessionFields};
use crate::app::{AppEvent, AppModel, AppState, Dispatcher};

use super::playback_widget::PlaybackWidget;
use super::{BarTrack, QueueBarWidget};

pub struct PlaybackModel {
    app_model: Rc<AppModel>,
    dispatcher: Dispatcher,
}

impl PlaybackModel {
    pub fn new(app_model: Rc<AppModel>, dispatcher: Dispatcher) -> Self {
        Self {
            app_model,
            dispatcher,
        }
    }

    fn state(&self) -> impl Deref<Target = AppState> + '_ {
        self.app_model.get_state()
    }

    fn api_service(&self) -> std::sync::Arc<riff_api::ApiService> {
        self.app_model.api()
    }

    fn go_home(&self) {
        self.dispatcher
            .dispatch(NavigationAction::Go(Route::Home(HomePage::NowPlaying)).into());
    }

    fn is_playing(&self) -> bool {
        self.state().playback.is_playing()
    }

    fn is_shuffled(&self) -> bool {
        self.state().playback.is_shuffled()
    }

    fn current_song(&self) -> Option<Track> {
        self.app_model.get_state().playback.current_song()
    }

    fn next_song(&self) -> Option<Track> {
        self.app_model.get_state().playback.next_song()
    }

    fn play_next_song(&self) {
        self.dispatcher.dispatch(PlaybackAction::Next.into());
    }

    fn play_prev_song(&self) {
        self.dispatcher.dispatch(PlaybackAction::Previous.into());
    }

    fn toggle_playback(&self) {
        self.dispatcher.dispatch(PlaybackAction::TogglePlay.into());
    }

    fn toggle_shuffle(&self) {
        self.dispatcher
            .dispatch(PlaybackAction::ToggleShuffle.into());
    }

    fn toggle_repeat(&self) {
        self.dispatcher
            .dispatch(PlaybackAction::ToggleRepeat.into());
    }

    fn seek_to(&self, position: u32) {
        self.dispatcher
            .dispatch(PlaybackAction::Seek(position).into());
    }

    fn set_volume(&self, value: f64) {
        self.dispatcher
            .dispatch(PlaybackAction::SetVolume(value).into())
    }
}

pub struct PlaybackControl {
    model: Rc<PlaybackModel>,
    widget: PlaybackWidget,
    queue_bar: QueueBarWidget,
}

impl PlaybackControl {
    pub fn new(model: PlaybackModel, widget: PlaybackWidget, queue_bar: QueueBarWidget) -> Self {
        let model = Rc::new(model);

        widget.connect_play_pause(clone!(
            #[weak]
            model,
            move || model.toggle_playback()
        ));
        widget.connect_next(clone!(
            #[weak]
            model,
            move || model.play_next_song()
        ));
        widget.connect_prev(clone!(
            #[weak]
            model,
            move || model.play_prev_song()
        ));
        widget.connect_shuffle(clone!(
            #[weak]
            model,
            move || model.toggle_shuffle()
        ));
        widget.connect_repeat(clone!(
            #[weak]
            model,
            move || model.toggle_repeat()
        ));
        widget.connect_seek(clone!(
            #[weak]
            model,
            move |position| model.seek_to(position)
        ));
        widget.connect_now_playing_clicked(clone!(
            #[weak]
            model,
            move || model.go_home()
        ));
        widget.connect_volume_changed(clone!(
            #[weak]
            model,
            move |value| model.set_volume(value)
        ));

        Self {
            model,
            widget,
            queue_bar,
        }
    }

    fn update_queue_bar(&self) {
        let bar_track = |song: Track| BarTrack {
            artist: song.artists_name(),
            title: song.title,
        };
        let now_playing = self.model.current_song().map(bar_track);
        let up_next = self.model.next_song().map(bar_track);
        self.queue_bar.set_tracks(now_playing, up_next);
    }

    fn update_repeat(&self, mode: &RepeatMode) {
        self.widget.set_repeat_mode(*mode);
    }

    fn update_shuffled(&self) {
        self.widget.set_shuffled(self.model.is_shuffled());
    }

    fn update_playing(&self) {
        let is_playing = self.model.is_playing();
        self.widget.set_playing(is_playing);
        self.queue_bar.set_paused(!is_playing);
    }

    fn update_current_info(&self) {
        if let Some(song) = self.model.current_song() {
            self.widget
                .set_title_and_artist(&song.title, &song.artists_name());
            self.widget.set_song_duration(Some(song.duration_ms as f64));
            if let Some(url) = song.art.best_for_width(120) {
                self.widget
                    .set_artwork_from_url(url.to_owned(), self.model.api_service());
            }
        } else {
            self.widget.reset_info();
        }
    }

    fn sync_seek(&self, pos: u32) {
        self.widget.set_seek_position(pos as f64);
    }

    fn set_connection_lost(&self, lost: bool) {
        if !lost && self.model.is_playing() {
            self.widget.resume_seek_position();
        }
    }
}

impl EventListener for PlaybackControl {
    fn on_event(&mut self, event: &AppEvent) {
        if let Some(playback) = event.playback() {
            let changed = playback.changed;
            if changed.intersects(PlaybackFields::STATUS | PlaybackFields::TRACK) {
                self.update_playing();
            }
            if changed.contains(PlaybackFields::TRACK) {
                self.update_current_info();
            }
            if changed.contains(PlaybackFields::REPEAT) {
                self.update_repeat(&playback.state.repeat);
            }
            if changed.contains(PlaybackFields::SHUFFLE) {
                self.update_shuffled();
            }
            if changed.contains(PlaybackFields::POSITION) {
                self.sync_seek(playback.state.position);
            }
            if changed.contains(PlaybackFields::VOLUME) {
                self.widget.set_volume(playback.state.volume);
            }
            if changed.intersects(
                PlaybackFields::TRACK | PlaybackFields::REPEAT | PlaybackFields::SHUFFLE,
            ) {
                self.update_queue_bar();
            }
        }
        match event {
            _ if event.queue_changed() => self.update_queue_bar(),
            AppEvent::Core(Event::Selection(selection)) if selection.mode_changed() => {
                self.widget.set_seekbar_visible(!selection.active);
            }
            AppEvent::Core(Event::Session(session))
                if session.changed.contains(SessionFields::CONNECTION) =>
            {
                self.set_connection_lost(session.connection_lost);
            }
            _ => {}
        }
    }
}
