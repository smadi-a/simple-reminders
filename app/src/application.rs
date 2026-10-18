//! The application object: wires together the `core` `Store`, the live poller, the
//! login/missed sweep, the "reminder-done" action, and (when a window is wanted) the GUI.
//!
//! See the project plan §6 for the single-instance design this implements: `connect_startup`
//! does all the one-time setup shared by both modes (it fires once per *primary* instance,
//! whether that instance was reached via `run()` or via a bare `register()` call). The
//! `connect_activate` handler — registered from inside `connect_startup`, once the shared
//! state exists — is the only place a window is ever created, so background-mode startup
//! (which never calls `run()`, so `activate` is never locally emitted) never shows one,
//! while every app-grid click (which does go through `run()`/`activate`) does.

use crate::audio::play_sound;
use crate::notifications::{parse_notification_id, send_reminder_notification};
use crate::ui::window::ReminderWindow;
use gio::prelude::*;
use simple_reminders_core::{scheduler, Store};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub const APP_ID: &str = "io.github.alielsmadi.SimpleReminders";

/// How often the background poller checks for newly-due reminders.
const POLL_INTERVAL_SECS: u32 = 25;

#[derive(Clone)]
struct AppState {
    store: Rc<Store>,
    window: Rc<RefCell<Option<ReminderWindow>>>,
}

pub fn build_application() -> adw::Application {
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_startup(|app| {
        load_css();

        let db_path = simple_reminders_core::paths::db_path().expect("failed to resolve data directory");
        let store = Rc::new(Store::open(&db_path).expect("failed to open reminders database"));

        let state = AppState { store: store.clone(), window: Rc::new(RefCell::new(None)) };

        // Run the missed-reminders sweep once at startup (covers both "logged out, now
        // logging back in" and "first launch ever") before anything else touches the DB.
        let now = chrono::Local::now().naive_local();
        match scheduler::run_missed_sweep(&store, now) {
            Ok(missed) => {
                for occurrence in missed {
                    let Ok(reminder) = store.get_reminder(occurrence.reminder_id) else { continue };
                    send_reminder_notification(app, occurrence.reminder_id, &reminder.title, occurrence.due_instant);
                    if let Some(sound_path) = sound_setting(&store) {
                        play_sound(&sound_path);
                    }
                }
            }
            Err(err) => eprintln!("missed-reminder sweep failed: {err}"),
        }

        register_reminder_done_action(app, state.clone());
        register_reminder_snooze_action(app, state.clone());
        start_poll_timer(app.clone(), state.clone());

        let activate_state = state.clone();
        app.connect_activate(move |app| {
            present_window(app, &activate_state);
        });
    });

    app
}

/// Runs this process as the headless background instance: makes it the primary instance
/// if none exists yet (which synchronously fires `startup` above), then blocks forever on
/// a plain GLib main loop with no window ever created. If a primary instance already
/// exists, this is a no-op — exactly one background scheduler ever runs at a time.
pub fn run_background(app: &adw::Application) -> glib::ExitCode {
    match app.register(gio::Cancellable::NONE) {
        Ok(()) => {}
        Err(err) => {
            eprintln!("failed to register application: {err}");
            return glib::ExitCode::FAILURE;
        }
    }

    if app.is_remote() {
        // Another instance (background or GUI) is already primary; nothing to do.
        return glib::ExitCode::SUCCESS;
    }

    // `hold()` returns an RAII guard that releases the hold when dropped — keep it bound
    // for as long as the main loop runs, which for this process is forever.
    let _hold_guard = app.hold();
    glib::MainLoop::new(None, false).run();
    glib::ExitCode::SUCCESS
}

fn present_window(app: &adw::Application, state: &AppState) {
    let mut window_slot = state.window.borrow_mut();
    if let Some(existing) = window_slot.as_ref() {
        existing.present();
    } else {
        let window = ReminderWindow::new(app, state.store.clone());
        window.present();
        *window_slot = Some(window);
    }
}

fn register_reminder_done_action(app: &adw::Application, state: AppState) {
    let action = gio::SimpleAction::new("reminder-done", Some(glib::VariantTy::STRING));
    let app_for_action = app.clone();
    action.connect_activate(move |_action, parameter| {
        let Some(notification_id) = parameter.and_then(|v| v.str().map(str::to_owned)) else {
            return;
        };
        let Some((reminder_id, _due_instant)) = parse_notification_id(&notification_id) else {
            return;
        };
        if let Err(err) = scheduler::mark_done(&state.store, reminder_id) {
            eprintln!("failed to mark reminder {reminder_id} done: {err}");
        }
        app_for_action.withdraw_notification(&notification_id);
        if let Some(window) = state.window.borrow().as_ref() {
            window.refresh();
        }
    });
    app.add_action(&action);
}

/// Registers the "Snooze" notification action: withdraws the current notification, marks
/// the reminder `snoozed_until` (so the main window can show it as snoozed in the
/// meantime), and after the configured delay re-sends it (same notification id, since it's
/// derived from the reminder id and due instant, which don't change) so the "Done" button
/// still resolves to the right occurrence. Re-reads the reminder from the store when the
/// delay elapses rather than capturing its title now, so a snooze outlives edits or
/// deletion made to the reminder in the meantime.
fn register_reminder_snooze_action(app: &adw::Application, state: AppState) {
    let action = gio::SimpleAction::new("reminder-snooze", Some(glib::VariantTy::STRING));
    let app_for_action = app.clone();
    action.connect_activate(move |_action, parameter| {
        let Some(notification_id) = parameter.and_then(|v| v.str().map(str::to_owned)) else {
            return;
        };
        let Some((reminder_id, due_instant)) = parse_notification_id(&notification_id) else {
            return;
        };
        app_for_action.withdraw_notification(&notification_id);

        let snooze_minutes = state.store.snooze_minutes().unwrap_or(5).max(1);
        let snoozed_until = chrono::Local::now().naive_local() + chrono::Duration::minutes(snooze_minutes);
        if let Err(err) = state.store.set_snoozed(reminder_id, snoozed_until) {
            eprintln!("failed to mark reminder {reminder_id} snoozed: {err}");
        }
        if let Some(window) = state.window.borrow().as_ref() {
            window.refresh();
        }

        let app = app_for_action.clone();
        let state = state.clone();
        glib::source::timeout_add_seconds_local(u32::try_from(snooze_minutes * 60).unwrap_or(u32::MAX), move || {
            if let Err(err) = state.store.clear_snoozed(reminder_id) {
                eprintln!("failed to clear snoozed state for reminder {reminder_id}: {err}");
            }
            if let Ok(reminder) = state.store.get_reminder(reminder_id) {
                send_reminder_notification(&app, reminder_id, &reminder.title, due_instant);
                if let Some(sound_path) = sound_setting(&state.store) {
                    play_sound(&sound_path);
                }
            }
            if let Some(window) = state.window.borrow().as_ref() {
                window.refresh();
            }
            glib::ControlFlow::Break
        });
    });
    app.add_action(&action);
}

fn start_poll_timer(app: adw::Application, state: AppState) {
    glib::source::timeout_add_seconds_local(POLL_INTERVAL_SECS, move || {
        let now = chrono::Local::now().naive_local();
        match scheduler::live_poll_step(&state.store, now) {
            Ok(fired) => {
                for reminder in fired {
                    send_reminder_notification(&app, reminder.reminder_id, &reminder.title, reminder.due_instant);
                    if let Some(sound_path) = sound_setting(&state.store) {
                        play_sound(&sound_path);
                    }
                }
                if let Err(err) = scheduler::record_last_seen(&state.store, now) {
                    eprintln!("failed to record last-seen timestamp: {err}");
                }
                if let Some(window) = state.window.borrow().as_ref() {
                    window.refresh();
                }
            }
            Err(err) => eprintln!("live poll step failed: {err}"),
        }
        glib::ControlFlow::Continue
    });
}

fn sound_setting(store: &Store) -> Option<PathBuf> {
    store.get_setting("notification_sound_path").ok().flatten().map(PathBuf::from)
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../../data/style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("no display"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
