mod application;
mod audio;
mod notifications;
mod ui;

use gtk::prelude::*;

fn main() -> glib::ExitCode {
    // Parsed before any GTK/GIO initialization, per the single-instance design in the
    // project plan §6: background mode never calls `Application::run()`.
    let background_mode = std::env::args().skip(1).any(|arg| arg == "--background");

    let app = application::build_application();

    if background_mode {
        application::run_background(&app)
    } else {
        app.run()
    }
}
