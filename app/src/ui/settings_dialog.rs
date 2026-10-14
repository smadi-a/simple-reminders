//! Settings dialog: lets the user pick which existing Ubuntu/GNOME system sound plays
//! when a reminder fires, with a preview button, and toggle auto-arranging the reminders
//! list by date/time. Selections are persisted immediately.

use crate::audio::play_sound;
use gtk::prelude::*;
use adw::prelude::*;
use simple_reminders_core::sound::{default_sound_dirs, discover_system_sounds_in};
use simple_reminders_core::{scheduler, Store};
use std::rc::Rc;

/// `on_auto_sort_changed` fires whenever the auto-arrange toggle flips, so the caller
/// (the main window) can rebuild its list immediately — both to reflect the sort just
/// applied when turning it on, and to show/hide the drag handles either way.
pub fn show_settings_dialog(
    parent: &impl IsA<gtk::Widget>,
    store: Rc<Store>,
    on_auto_sort_changed: impl Fn() + 'static,
) {
    let dialog = adw::PreferencesDialog::new();
    let page = adw::PreferencesPage::new();
    // Keep the dialog auto-sizing to its content, but enforce a minimum height so it
    // doesn't feel cramped when there are few rows.
    page.set_size_request(-1, 300);

    let arrangement_group = adw::PreferencesGroup::builder().title("Arrangement").build();
    let auto_sort_row = adw::SwitchRow::builder()
        .title("Auto-Arrange by Date &amp; Time")
        .subtitle("Keeps reminders sorted soonest-first and disables manual drag-to-reorder")
        .build();
    auto_sort_row.set_active(store.auto_sort_enabled().unwrap_or(false));
    {
        let store = store.clone();
        auto_sort_row.connect_active_notify(move |row| {
            let enabled = row.is_active();
            if let Err(err) = store.set_auto_sort_enabled(enabled) {
                eprintln!("failed to save auto-sort setting: {err}");
            }
            if enabled {
                let now = chrono::Local::now().naive_local();
                if let Err(err) = scheduler::apply_auto_sort(&store, now) {
                    eprintln!("failed to auto-sort reminders: {err}");
                }
            }
            on_auto_sort_changed();
        });
    }
    arrangement_group.add(&auto_sort_row);
    page.add(&arrangement_group);

    let group = adw::PreferencesGroup::builder().title("Notification Sound").build();

    let sounds = discover_system_sounds_in(&default_sound_dirs());

    if sounds.is_empty() {
        let row = adw::ActionRow::builder().title("No system sounds found").build();
        row.set_sensitive(false);
        group.add(&row);
    } else {
        let labels: Vec<String> = sounds
            .iter()
            .map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())
            .collect();
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let model = gtk::StringList::new(&label_refs);
        let combo = adw::ComboRow::builder().title("Sound").model(&model).build();

        let current_path = store.get_setting("notification_sound_path").ok().flatten();
        let selected_index = current_path
            .and_then(|current| sounds.iter().position(|p| p.to_string_lossy() == current))
            .unwrap_or(0);
        combo.set_selected(selected_index as u32);

        let play_button = gtk::Button::from_icon_name("media-playback-start-symbolic");
        play_button.set_valign(gtk::Align::Center);
        play_button.add_css_class("flat");
        play_button.set_tooltip_text(Some("Preview"));
        combo.add_suffix(&play_button);

        {
            let sounds = sounds.clone();
            let combo = combo.clone();
            play_button.connect_clicked(move |_| {
                if let Some(path) = sounds.get(combo.selected() as usize) {
                    play_sound(path);
                }
            });
        }

        {
            let sounds = sounds.clone();
            let store = store.clone();
            combo.connect_selected_notify(move |row| {
                if let Some(path) = sounds.get(row.selected() as usize) {
                    if let Err(err) = store.set_setting("notification_sound_path", &path.to_string_lossy()) {
                        eprintln!("failed to save sound setting: {err}");
                    }
                }
            });
        }

        // Persist the default selection immediately if nothing was chosen yet, so a
        // reminder firing before the user ever opens Settings still has a sound to play.
        if store.get_setting("notification_sound_path").ok().flatten().is_none() {
            if let Some(path) = sounds.first() {
                let _ = store.set_setting("notification_sound_path", &path.to_string_lossy());
            }
        }

        group.add(&combo);
    }

    page.add(&group);
    dialog.add(&page);
    dialog.present(Some(parent));
}
