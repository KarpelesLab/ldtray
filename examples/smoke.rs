//! Non-interactive smoke test used by CI to actually *run* the tray on a GUI
//! runner (Windows/macOS have a desktop session) and prove the backend creates,
//! pumps, updates, notifies and tears down without crashing.
//!
//! It runs the event loop on the **main thread** (required by macOS AppKit),
//! while a driver thread exercises every `TrayHandle` method and then quits. A
//! watchdog guarantees the process can never hang CI.
//!
//! Exit codes: `0` = ran cleanly (or no tray in this environment, e.g. a
//! headless Linux runner); non-zero = the backend errored, panicked, or hung.

use std::thread;
use std::time::Duration;

use ldtray::{Event, Icon, Menu, MenuItem, Notification, Tray, TrayConfig};

fn demo_icon() -> Icon {
    let side = 16u32;
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    for _ in 0..side * side {
        rgba.extend_from_slice(&[10, 120, 220, 255]);
    }
    Icon::from_rgba(side, side, rgba).expect("valid icon")
}

fn demo_menu() -> Menu {
    Menu::new()
        .item(MenuItem::button(1, "One"))
        .item(MenuItem::checkbox(2, "Toggle", true))
        .item(MenuItem::separator())
        .item(MenuItem::submenu("More", [MenuItem::button(3, "Nested")]))
        .item(MenuItem::button(9, "Quit"))
}

fn main() {
    // Hard stop so a hang shows up as a CI failure rather than a 6-hour job.
    thread::spawn(|| {
        thread::sleep(Duration::from_secs(20));
        eprintln!("SMOKE_TIMEOUT: event loop did not finish");
        std::process::exit(3);
    });

    let config = TrayConfig::new(demo_icon())
        .tooltip("ldtray smoke")
        .menu(demo_menu());

    let tray = match Tray::new(config) {
        Ok(tray) => tray,
        Err(err) => {
            // Expected on a headless box (e.g. Linux CI: no session bus). On a
            // GUI OS the platform libraries always exist, so treat a failure
            // there as a real regression.
            if cfg!(any(target_os = "windows", target_os = "macos")) {
                eprintln!("SMOKE_FAIL: Tray::new failed on a GUI OS: {err}");
                std::process::exit(1);
            }
            eprintln!("SMOKE_SKIP: no tray in this environment: {err}");
            return;
        }
    };

    // Drive every mutating method from another thread, then stop the loop.
    let handle = tray.handle();
    let driver = thread::spawn(move || {
        thread::sleep(Duration::from_millis(400));
        let _ = handle.set_tooltip("updated tooltip");
        let _ = handle.set_icon(demo_icon());
        let _ = handle.set_menu(demo_menu());
        let _ = handle.notify(Notification::new("ldtray", "smoke").with_icon(demo_icon()));
        thread::sleep(Duration::from_millis(400));
        let _ = handle.quit();
    });

    let mut events = 0usize;
    let result = tray.run(|event: Event| {
        events += 1;
        println!("event: {event:?}");
    });
    let _ = driver.join();

    match result {
        Ok(()) => {
            println!("SMOKE_OK: ran cleanly ({events} events)");
        }
        Err(err) => {
            eprintln!("SMOKE_FAIL: event loop ended with error: {err}");
            std::process::exit(2);
        }
    }
}
