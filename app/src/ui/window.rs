//! The main window: a "Missed" section (dark red, only visible when non-empty) and a
//! "Reminders" section, both lists of `adw::ActionRow`. All validation/formatting/
//! persistence lives in `core`; this module only builds widgets and forwards calls.

use crate::ui::edit_dialog;
use crate::ui::settings_dialog;
use gtk::prelude::*;
use adw::prelude::*;
use simple_reminders_core::{format::describe_recurrence, scheduler, Reminder, Store};
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
            .default_width(480)
            .default_height(640)
            .content(&toolbar_view)
            .build();

        window.connect_close_request(|window| {
            // Hide, don't destroy: scheduling/notifications must keep working after the
            // window closes (see the project plan §6) — there is deliberately no quit
            // action anywhere in this app.
            window.set_visible(false);
            glib::Propagation::Stop
        });

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
                settings_dialog::show_settings_dialog(&this.0.window, this.0.store.clone());
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
            for reminder in &normal {
                group.add(&self.build_reminder_row(reminder));
            }
        }
        self.0.content_box.append(&group);
        *self.0.reminders_group.borrow_mut() = Some(group);
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

    fn build_reminder_row(&self, reminder: &Reminder) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(&reminder.title)
            .subtitle(describe_recurrence(&reminder.recurrence, reminder.start_date, reminder.time_of_day))
            .build();

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
