//! The main window: a "Missed" section (dark red, only visible when non-empty) and a
//! "Reminders" section, both lists of `adw::ActionRow`. All validation/formatting/
//! persistence lives in `core`; this module only builds widgets and forwards calls.

use crate::ui::edit_dialog;
use crate::ui::settings_dialog;
use gio::prelude::*;
use gtk::gdk;
use gtk::prelude::*;
use adw::prelude::*;
use simple_reminders_core::{format::describe_recurrence, model::move_id, scheduler, Reminder, Store};
use std::cell::RefCell;
use std::rc::Rc;

struct Inner {
    window: adw::ApplicationWindow,
    store: Rc<Store>,
    content_box: gtk::Box,
    missed_group: RefCell<Option<adw::PreferencesGroup>>,
    reminders_group: RefCell<Option<adw::PreferencesGroup>>,
}

#[derive(Clone)]
pub struct ReminderWindow(Rc<Inner>);

impl ReminderWindow {
    pub fn new(app: &adw::Application, store: Rc<Store>) -> Self {
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&adw::WindowTitle::new("Simple Reminders", "")));

        let add_button = gtk::Button::from_icon_name("list-add-symbolic");
        add_button.set_tooltip_text(Some("Add Reminder"));
        header.pack_start(&add_button);

        let settings_button = gtk::Button::from_icon_name("emblem-system-symbolic");
        settings_button.set_tooltip_text(Some("Settings"));
        header.pack_end(&settings_button);

        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content_box.set_margin_top(18);
        content_box.set_margin_bottom(18);
        content_box.set_margin_start(18);
        content_box.set_margin_end(18);

        let scrolled = gtk::ScrolledWindow::new();
        scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
        scrolled.set_vexpand(true);
        scrolled.set_child(Some(&content_box));

        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&scrolled));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Simple Reminders")
            .default_width(520)
            .default_height(640)
            .width_request(520)
            .content(&toolbar_view)
            .build();

        window.connect_close_request(|window| {
            // Hide, don't destroy: scheduling/notifications must keep working after the
            // window closes (see the project plan §6) — there is deliberately no action
            // anywhere in this app that quits the underlying process.
            window.set_visible(false);
            glib::Propagation::Stop
        });

        // Ctrl+Q closes just the window (same as the titlebar close button, via
        // close_request above) — it must never quit the process, since this same
        // process keeps running the scheduler/notifications after the window hides.
        let quit_ui_action = gio::SimpleAction::new("quit-ui", None);
        {
            let window = window.clone();
            quit_ui_action.connect_activate(move |_, _| {
                window.close();
            });
        }
        window.add_action(&quit_ui_action);
        app.set_accels_for_action("win.quit-ui", &["<Primary>q"]);

        let this = ReminderWindow(Rc::new(Inner {
            window,
            store,
            content_box,
            missed_group: RefCell::new(None),
            reminders_group: RefCell::new(None),
        }));

        {
            let this = this.clone();
            add_button.connect_clicked(move |_| {
                let this2 = this.clone();
                edit_dialog::show_edit_dialog(&this.0.window, this.0.store.clone(), None, move || this2.refresh());
            });
        }
        {
            let this = this.clone();
            settings_button.connect_clicked(move |_| {
                let this2 = this.clone();
                settings_dialog::show_settings_dialog(&this.0.window, this.0.store.clone(), move || this2.refresh());
            });
        }

        this.refresh();
        this
    }

    pub fn present(&self) {
        self.0.window.present();
    }

    pub fn refresh(&self) {
        if let Some(group) = self.0.missed_group.borrow_mut().take() {
            self.0.content_box.remove(&group);
        }
        if let Some(group) = self.0.reminders_group.borrow_mut().take() {
            self.0.content_box.remove(&group);
        }

        let auto_sort_enabled = self.0.store.auto_sort_enabled().unwrap_or(false);
        if auto_sort_enabled {
            // Reassessed on every launch/add/edit/removal, since those are exactly the
            // moments `refresh()` is already called from.
            if let Err(err) = scheduler::apply_auto_sort(&self.0.store, chrono::Local::now().naive_local()) {
                eprintln!("failed to auto-sort reminders: {err}");
            }
        }

        let reminders = match self.0.store.list_reminders() {
            Ok(r) => r,
            Err(err) => {
                eprintln!("failed to list reminders: {err}");
                return;
            }
        };

        let (missed, normal): (Vec<Reminder>, Vec<Reminder>) = reminders.into_iter().partition(|r| r.missed);

        if !missed.is_empty() {
            let group = adw::PreferencesGroup::builder().title("Missed").build();
            for reminder in &missed {
                group.add(&self.build_missed_row(reminder));
            }
            self.0.content_box.append(&group);
            *self.0.missed_group.borrow_mut() = Some(group);
        }

        let group = adw::PreferencesGroup::builder().title("Reminders").build();
        if normal.is_empty() {
            let placeholder = adw::ActionRow::builder().title("No reminders yet — tap + to add one").build();
            placeholder.set_sensitive(false);
            group.add(&placeholder);
        } else {
            let order: Rc<Vec<i64>> = Rc::new(normal.iter().map(|r| r.id).collect());
            for reminder in &normal {
                group.add(&self.build_reminder_row(reminder, &order, auto_sort_enabled));
            }
        }
        self.0.content_box.append(&group);
        *self.0.reminders_group.borrow_mut() = Some(group);
    }

    /// Applies a manual drag-to-reorder: moves `dragged_id` to just before/after
    /// `target_id` (see `model::move_id`), persists the result, then rebuilds the UI from
    /// the store (simplest way to reflect the new `ORDER BY sort_order` row order).
    fn reorder_reminder(&self, order: &[i64], dragged_id: i64, target_id: i64, insert_after: bool) {
        let new_order = move_id(order, dragged_id, target_id, insert_after);
        if let Err(err) = self.0.store.reorder_reminders(&new_order) {
            eprintln!("failed to reorder reminders: {err}");
        }
        self.refresh();
    }

    fn build_missed_row(&self, reminder: &Reminder) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(&reminder.title)
            .subtitle(describe_recurrence(&reminder.recurrence, reminder.start_date, reminder.time_of_day))
            .build();
        row.add_css_class("missed-reminder");

        let done_button = gtk::Button::with_label("Mark as Done");
        done_button.set_valign(gtk::Align::Center);
        done_button.add_css_class("flat");

        let this = self.clone();
        let reminder_id = reminder.id;
        done_button.connect_clicked(move |_| {
            if let Err(err) = scheduler::mark_done(&this.0.store, reminder_id) {
                eprintln!("failed to mark reminder {reminder_id} done: {err}");
            }
            this.refresh();
        });

        row.add_suffix(&done_button);
        row
    }

    fn build_reminder_row(&self, reminder: &Reminder, order: &Rc<Vec<i64>>, auto_sort_enabled: bool) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(&reminder.title)
            .subtitle(describe_recurrence(&reminder.recurrence, reminder.start_date, reminder.time_of_day))
            .build();

        // Manual drag-to-reorder only makes sense when the list's order isn't being
        // recomputed out from under the user — auto-arrange and manual order are mutually
        // exclusive, so skip the drag handle/source/target entirely while it's on.
        if !auto_sort_enabled {
            // Drag handle: only the handle itself is a drag source (not the whole row), so
            // clicking the row still opens it for editing instead of starting a drag.
            let handle = gtk::Image::from_icon_name("list-drag-handle-symbolic");
            handle.set_tooltip_text(Some("Drag to reorder"));
            handle.add_css_class("dim-label");
            handle.set_cursor_from_name(Some("grab"));

            let drag_source = gtk::DragSource::new();
            drag_source.set_actions(gdk::DragAction::MOVE);
            let dragged_id = reminder.id;
            drag_source.connect_prepare(move |_, _, _| Some(gdk::ContentProvider::for_value(&dragged_id.to_value())));
            handle.add_controller(drag_source);

            // The drop target spans the whole row so releasing anywhere over it works; which
            // half of the row the pointer is over decides "insert before" vs "insert after".
            let drop_target = gtk::DropTarget::new(glib::types::Type::I64, gdk::DragAction::MOVE);
            {
                let this = self.clone();
                let order = order.clone();
                let target_id = reminder.id;
                drop_target.connect_drop(move |drop_target, value, _x, y| {
                    let Ok(dragged_id) = value.get::<i64>() else {
                        return false;
                    };
                    if dragged_id == target_id {
                        return false;
                    }
                    let insert_after = drop_target
                        .widget()
                        .map(|widget| y > widget.height() as f64 / 2.0)
                        .unwrap_or(false);
                    this.reorder_reminder(&order, dragged_id, target_id, insert_after);
                    true
                });
            }
            row.add_controller(drop_target);
            row.add_prefix(&handle);
        }

        let edit_button = gtk::Button::from_icon_name("document-edit-symbolic");
        edit_button.set_valign(gtk::Align::Center);
        edit_button.add_css_class("flat");
        edit_button.set_tooltip_text(Some("Edit"));

        let delete_button = gtk::Button::from_icon_name("user-trash-symbolic");
        delete_button.set_valign(gtk::Align::Center);
        delete_button.add_css_class("flat");
        delete_button.set_tooltip_text(Some("Delete"));

        {
            let this = self.clone();
            let reminder = reminder.clone();
            edit_button.connect_clicked(move |_| {
                let this2 = this.clone();
                edit_dialog::show_edit_dialog(&this.0.window, this.0.store.clone(), Some(reminder.clone()), move || this2.refresh());
            });
        }
        {
            let this = self.clone();
            let reminder_id = reminder.id;
            delete_button.connect_clicked(move |_| {
                if let Err(err) = this.0.store.delete_reminder(reminder_id) {
                    eprintln!("failed to delete reminder {reminder_id}: {err}");
                }
                this.refresh();
            });
        }

        row.add_suffix(&edit_button);
        row.add_suffix(&delete_button);
        row
    }
}

impl std::ops::Deref for ReminderWindow {
    type Target = adw::ApplicationWindow;

    fn deref(&self) -> &Self::Target {
        &self.0.window
    }
}
