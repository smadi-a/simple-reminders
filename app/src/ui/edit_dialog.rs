//! Add/Edit reminder dialog. Validates input then delegates straight to `core::db::Store`;
//! no business logic lives here.

use gtk::prelude::*;
use adw::prelude::*;
use chrono::{Datelike, NaiveDate, NaiveTime, Weekday};
use simple_reminders_core::{NewReminder, RecurrenceRule, Reminder, Store};
use std::rc::Rc;

const RECURRENCE_LABELS: [&str; 5] = ["One-time", "Daily", "Weekdays", "Weekly", "Monthly"];
const WEEKDAY_ORDER: [(Weekday, &str); 7] = [
    (Weekday::Mon, "Mon"),
    (Weekday::Tue, "Tue"),
    (Weekday::Wed, "Wed"),
    (Weekday::Thu, "Thu"),
    (Weekday::Fri, "Fri"),
    (Weekday::Sat, "Sat"),
    (Weekday::Sun, "Sun"),
];

fn recurrence_index(rule: &RecurrenceRule) -> u32 {
    match rule {
        RecurrenceRule::OneTime => 0,
        RecurrenceRule::Daily => 1,
        RecurrenceRule::Weekdays => 2,
        RecurrenceRule::Weekly { .. } => 3,
        RecurrenceRule::MonthlyOnDay { .. } => 4,
    }
}

fn glib_date_to_naive(date: &glib::DateTime) -> NaiveDate {
    NaiveDate::from_ymd_opt(date.year(), date.month() as u32, date.day_of_month() as u32)
        .unwrap_or_else(|| chrono::Local::now().naive_local().date())
}

fn naive_to_glib_date(date: NaiveDate) -> glib::DateTime {
    glib::DateTime::new(
        &glib::TimeZone::local(),
        date.year(),
        date.month() as i32,
        date.day() as i32,
        0,
        0,
        0.0,
    )
    .expect("valid calendar date")
}

pub fn show_edit_dialog(
    parent: &impl IsA<gtk::Widget>,
    store: Rc<Store>,
    existing: Option<Reminder>,
    on_saved: impl Fn() + 'static,
) {
    let is_edit = existing.is_some();
    let today = chrono::Local::now().naive_local().date();

    let title_row = adw::EntryRow::builder().title("Title").build();
    if let Some(r) = &existing {
        title_row.set_text(&r.title);
    }

    let day_button = gtk::Button::new();
    let calendar = gtk::Calendar::new();
    let selected_date = existing.as_ref().map(|r| r.start_date).unwrap_or(today);
    calendar.select_day(&naive_to_glib_date(selected_date));
    day_button.set_label(&selected_date.format("%b %-d, %Y").to_string());

    let popover = gtk::Popover::new();
    popover.set_child(Some(&calendar));
    popover.set_parent(&day_button);
    {
        let day_button = day_button.clone();
        let popover_for_calendar = popover.clone();
        calendar.connect_day_selected(move |cal| {
            let date = glib_date_to_naive(&cal.date());
            day_button.set_label(&date.format("%b %-d, %Y").to_string());
            popover_for_calendar.popdown();
        });
    }
    day_button.connect_clicked(move |_| popover.popup());

    let day_row = adw::ActionRow::builder().title("Day").build();
    day_row.add_suffix(&day_button);

    let (initial_hour, initial_minute) = existing
        .as_ref()
        .map(|r| (r.time_of_day.format("%H").to_string().parse().unwrap_or(9), r.time_of_day.format("%M").to_string().parse().unwrap_or(0)))
        .unwrap_or((9, 0));

    let hour_spin = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    hour_spin.set_value(initial_hour as f64);
    hour_spin.set_wrap(true);
    let minute_spin = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
    minute_spin.set_value(initial_minute as f64);
    minute_spin.set_wrap(true);

    let time_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    time_box.append(&hour_spin);
    time_box.append(&gtk::Label::new(Some(":")));
    time_box.append(&minute_spin);

    let time_row = adw::ActionRow::builder().title("Time").build();
    time_row.add_suffix(&time_box);

    let recurrence_model = gtk::StringList::new(&RECURRENCE_LABELS);
    let recurrence_row = adw::ComboRow::builder().title("Repeat").model(&recurrence_model).build();
    let initial_recurrence_index = existing.as_ref().map(|r| recurrence_index(&r.recurrence)).unwrap_or(0);
    recurrence_row.set_selected(initial_recurrence_index);

    // Weekly day toggles.
    let weekday_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let existing_weekly_days: Vec<Weekday> = match &existing.as_ref().map(|r| &r.recurrence) {
        Some(RecurrenceRule::Weekly { days }) => days.clone(),
        _ => Vec::new(),
    };
    let day_toggles: Rc<Vec<(Weekday, gtk::ToggleButton)>> = Rc::new(
        WEEKDAY_ORDER
            .iter()
            .map(|(day, label)| {
                let button = gtk::ToggleButton::with_label(label);
                button.set_active(existing_weekly_days.contains(day));
                weekday_box.append(&button);
                (*day, button)
            })
            .collect(),
    );
    let weekly_revealer = gtk::Revealer::new();
    weekly_revealer.set_child(Some(&weekday_box));
    weekly_revealer.set_reveal_child(initial_recurrence_index == 3);

    // Monthly day spin button.
    let initial_monthly_day = match &existing.as_ref().map(|r| &r.recurrence) {
        Some(RecurrenceRule::MonthlyOnDay { day }) => *day,
        _ => 1,
    };
    let monthly_spin = gtk::SpinButton::with_range(1.0, 31.0, 1.0);
    monthly_spin.set_value(initial_monthly_day as f64);
    let monthly_row = adw::ActionRow::builder().title("Day of month").build();
    monthly_row.add_suffix(&monthly_spin);
    let monthly_revealer = gtk::Revealer::new();
    monthly_revealer.set_child(Some(&monthly_row));
    monthly_revealer.set_reveal_child(initial_recurrence_index == 4);

    {
        let weekly_revealer = weekly_revealer.clone();
        let monthly_revealer = monthly_revealer.clone();
        recurrence_row.connect_selected_notify(move |row| {
            let selected = row.selected();
            weekly_revealer.set_reveal_child(selected == 3);
            monthly_revealer.set_reveal_child(selected == 4);
        });
    }

    let error_label = gtk::Label::new(None);
    error_label.add_css_class("error");
    error_label.set_visible(false);
    error_label.set_wrap(true);

    let group = adw::PreferencesGroup::new();
    group.add(&title_row);
    group.add(&day_row);
    group.add(&time_row);
    group.add(&recurrence_row);

    let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
    page.set_margin_top(12);
    page.set_margin_bottom(12);
    page.set_margin_start(12);
    page.set_margin_end(12);
    page.append(&group);
    page.append(&weekly_revealer);
    page.append(&monthly_revealer);
    page.append(&error_label);

    let cancel_button = gtk::Button::with_label("Cancel");
    let save_button = gtk::Button::with_label(if is_edit { "Save" } else { "Add" });
    save_button.add_css_class("suggested-action");

    let action_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    action_box.set_halign(gtk::Align::End);
    action_box.append(&cancel_button);
    action_box.append(&save_button);

    let outer = gtk::Box::new(gtk::Orientation::Vertical, 6);
    outer.append(&page);
    outer.append(&action_box);
    outer.set_margin_bottom(12);
    outer.set_margin_start(12);
    outer.set_margin_end(12);

    let dialog = adw::Dialog::builder()
        .title(if is_edit { "Edit Reminder" } else { "Add Reminder" })
        .content_width(420)
        .child(&outer)
        .build();

    {
        let dialog = dialog.clone();
        cancel_button.connect_clicked(move |_| {
            dialog.close();
        });
    }

    let existing_id = existing.as_ref().map(|r| r.id);
    let dialog_for_save = dialog.clone();
    let day_toggles_for_save = day_toggles.clone();
    save_button.connect_clicked(move |_| {
        let title_text = title_row.text().to_string().trim().to_string();
        if title_text.is_empty() {
            error_label.set_text("Title can't be empty.");
            error_label.set_visible(true);
            return;
        }

        let start_date = glib_date_to_naive(&calendar.date());
        let hour = hour_spin.value() as u32;
        let minute = minute_spin.value() as u32;
        let time_of_day = NaiveTime::from_hms_opt(hour, minute, 0).unwrap();

        let recurrence = match recurrence_row.selected() {
            0 => RecurrenceRule::OneTime,
            1 => RecurrenceRule::Daily,
            2 => RecurrenceRule::Weekdays,
            3 => {
                let days: Vec<Weekday> =
                    day_toggles_for_save.iter().filter(|(_, button)| button.is_active()).map(|(day, _)| *day).collect();
                if days.is_empty() {
                    error_label.set_text("Pick at least one day for a weekly reminder.");
                    error_label.set_visible(true);
                    return;
                }
                RecurrenceRule::Weekly { days }
            }
            _ => RecurrenceRule::MonthlyOnDay { day: monthly_spin.value() as u8 },
        };

        let new_reminder = NewReminder { title: title_text, time_of_day, recurrence, start_date };

        let result = match existing_id {
            Some(id) => store.update_reminder(id, &new_reminder),
            None => store.insert_reminder(&new_reminder, chrono::Local::now().naive_local()).map(|_| ()),
        };

        match result {
            Ok(()) => {
                on_saved();
                dialog_for_save.close();
            }
            Err(err) => {
                error_label.set_text(&format!("Couldn't save: {err}"));
                error_label.set_visible(true);
            }
        }
    });

    dialog.present(Some(parent));
}
