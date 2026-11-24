use gtk::prelude::*;
use libadwaita::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

use super::PlaylistDetailsModel;

use crate::app::components::utils::{confirm_delete_playlist, decode_px};
use crate::app::components::{
    labels, Component, DetailsPageComponent, EventListener, HasHeaderBarModel, PageHeader,
    PlaylistDialog,
};
use crate::app::state::Route;
use crate::app::{AppEvent, ProvidesApi};
use riff_api::models::is_resource_url;
use riff_core::load;

const EDIT_DETAILS_ENTRY: &str = "edit_details";

const COVER_PREVIEW_SIZE: i32 = 160;

pub struct PlaylistDetails {
    model: Rc<PlaylistDetailsModel>,
    component: DetailsPageComponent<PlaylistDetailsModel>,
    owner_menu_added: Cell<bool>,
}

impl PlaylistDetails {
    pub fn new(model: Rc<PlaylistDetailsModel>, header: PageHeader) -> Self {
        let mut component =
            DetailsPageComponent::new(model.clone(), model.to_headerbar_model(), header);
        component.create_track_list(None);

        Self {
            model,
            component,
            owner_menu_added: Cell::new(false),
        }
    }

    fn add_owner_menu(&self) {
        if self.owner_menu_added.get() || !self.model.is_playlist_editable() {
            return;
        }
        self.owner_menu_added.set(true);

        self.component.add_menu_entry(
            EDIT_DETAILS_ENTRY,
            &labels::EDIT_PLAYLIST_DETAILS,
            clone!(
                #[weak(rename_to = model)]
                self.model,
                #[weak(rename_to = widget)]
                self.component.page().widget(),
                move || Self::show_edit_dialog(&model, &widget)
            ),
        );
        self.component.add_menu_entry(
            "delete",
            &labels::DELETE_PLAYLIST,
            clone!(
                #[weak(rename_to = model)]
                self.model,
                #[weak(rename_to = widget)]
                self.component.page().widget(),
                move || Self::confirm_delete(&model, &widget)
            ),
        );
    }

    fn show_edit_dialog(model: &Rc<PlaylistDetailsModel>, widget: &impl IsA<gtk::Widget>) {
        let Some((name, description, public, cover)) = model.get_playlist_info().map(|p| {
            let cover = p.art.best_for_width(COVER_PREVIEW_SIZE as u32);
            (
                p.title.clone(),
                p.description.clone(),
                p.public,
                cover.filter(|url| !is_resource_url(url)).map(str::to_owned),
            )
        }) else {
            return;
        };
        let dialog =
            PlaylistDialog::for_editing(&name, description.as_deref(), public == Some(true));
        if let Some(url) = cover {
            let api = model.api_service();
            let size = decode_px(COVER_PREVIEW_SIZE);
            let tag = load::visible();
            glib::spawn_future_local(clone!(
                #[weak]
                dialog,
                async move {
                    if let Some(texture) = api.load_image(&url, size, size, tag).await {
                        dialog.set_current_cover(&texture);
                    }
                }
            ));
        }
        dialog.connect_save(clone!(
            #[weak]
            model,
            #[upgrade_or_else]
            || tokio::sync::oneshot::channel().1,
            move |form| model.edit_playlist(form)
        ));
        dialog.present(Some(widget));
    }

    fn confirm_delete(model: &Rc<PlaylistDetailsModel>, widget: &impl IsA<gtk::Widget>) {
        confirm_delete_playlist(
            Some(widget.upcast_ref()),
            clone!(
                #[weak]
                model,
                move || model.delete_playlist()
            ),
        );
    }

    fn set_editing(&self, editing: bool) {
        if !self.model.is_playlist_editable() {
            return;
        }
        if !editing {
            let new_name = self.component.page().header().get_title_text();
            let info = self.model.get_playlist_info();
            if let Some(info) = info {
                if new_name != info.title && !new_name.is_empty() {
                    self.model.update_playlist_details(new_name);
                } else {
                    self.component.page().header().set_title(&info.title);
                }
            }
        }
    }
}

impl Component for PlaylistDetails {
    fn get_root_widget(&self) -> &gtk::Widget {
        self.component.get_root_widget()
    }
    fn get_children(&mut self) -> Option<&mut Vec<Box<dyn EventListener>>> {
        self.component.get_children()
    }
}

impl EventListener for PlaylistDetails {
    fn on_event(&mut self, event: &AppEvent) {
        self.component.handle_event(event);
        if let Some(selection) = event.selection().filter(|s| s.mode_changed()) {
            self.set_editing(selection.active);
        }
        if event
            .page_loaded(&Route::Playlist(self.model.id.clone()))
            .is_some()
        {
            self.add_owner_menu();
        }
        self.broadcast_event(event);
    }
}
