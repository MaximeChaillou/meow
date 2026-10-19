mod app;
mod ui;

use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

fn main() {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("meow must start on the main thread");
        std::process::exit(1);
    };
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon: the cat is the interface.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app::start(mtm);
    app.run();
}
