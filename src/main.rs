//! Services Manager: every service defined on this machine, what each is
//! doing, and the commands that start, stop, restart, reload and reset them.
//!
//! It asks peinit, on its control socket, and the registry, and it does what
//! peinit's control interface does, on behalf of whoever is looking: nothing
//! here has more authority than they have. What is running now, service or
//! not, is Task Manager's to show; what is defined is this.

use std::sync::{Arc, Weak};
use std::time::Duration;

use libgxwi::{App, Surface};

mod manager;
mod system;
mod words;

use manager::Manager;

// What this program looks like, to whatever lists it. The icon itself is
// `services-manager.svg` at the repo root, installed as the base theme's.
libgxwi::icon!(b"dev.peios.services-manager");

/// How often the window looks again at what the services are doing. peinit
/// says nothing when a service changes state, so the window asks.
const EVERY: Duration = Duration::from_secs(2);

/// Looks at the services, and at the one picked in full, every so often,
/// for as long as the window is there.
fn watch(window: Weak<Surface<Manager>>) {
    loop {
        let Some(picked) = window.upgrade().map(|window| window.look(|manager, _, _| manager.picked())) else { return };
        let seen = system::look();
        let status = picked.map(|picked| {
            let status = system::status(&picked);
            (picked, status)
        });
        let Some(window_now) = window.upgrade() else { return };
        window_now.update(|manager, _| {
            manager.seen(seen);
            if let Some((picked, status)) = status {
                manager.detailed(&picked, status);
            }
        });
        drop(window_now);
        std::thread::sleep(EVERY);
    }
}

fn main() {
    let mut app = match App::connect() {
        Ok(app) => app,
        Err(e) => {
            eprintln!("services-manager: no desktop to open on: {e}");
            eprintln!("services-manager: on a terminal, svctl does what this does");
            std::process::exit(1);
        }
    };
    app.stylesheet("/services-manager.css", include_str!("services-manager.css"));
    let window = app.live("Services Manager", Manager::new());
    let aside = Arc::downgrade(&window);
    window.update(|manager, _| manager.window = aside.clone());
    std::thread::spawn(move || watch(aside));
    if let Err(e) = app.run() {
        eprintln!("services-manager: {e}");
        std::process::exit(1);
    }
}
