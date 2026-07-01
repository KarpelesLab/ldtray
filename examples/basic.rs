//! Minimal end-to-end example.
//!
//! Shows a red tray icon with a small menu, prints every event, pops a
//! notification when "Say hi" is clicked, and exits on "Quit". If no tray is
//! available (headless server, missing libraries) it prints why and exits 0 —
//! demonstrating the graceful-degradation contract.

use ldtray::{Event, Icon, Menu, MenuItem, Notification, Tray, TrayConfig};

const SAY_HI: u32 = 1;
const TOGGLE: u32 = 2;
const QUIT: u32 = 3;

fn main() {
    // A 16x16 solid red icon.
    let side = 16u32;
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    for _ in 0..side * side {
        rgba.extend_from_slice(&[220, 40, 40, 255]);
    }
    let icon = Icon::from_rgba(side, side, rgba).expect("valid icon");

    let menu = Menu::new()
        .item(MenuItem::button(SAY_HI, "Say hi"))
        .item(MenuItem::checkbox(TOGGLE, "Toggle me", false))
        .item(MenuItem::separator())
        .item(MenuItem::button(QUIT, "Quit"));

    let config = TrayConfig::new(icon).tooltip("ldtray example").menu(menu);

    let tray = match Tray::new(config) {
        Ok(tray) => tray,
        Err(err) => {
            eprintln!("tray unavailable: {err}");
            eprintln!("(this is expected on a headless server — exiting cleanly)");
            return;
        }
    };

    let handle = tray.handle();
    println!("tray is up — right-click it for the menu, or Ctrl-C to abort");

    let result = tray.run(move |event| {
        println!("event: {event:?}");
        match event {
            Event::Menu(id) if id.0 == SAY_HI => {
                let _ = handle.notify(Notification::new("ldtray", "Hello from the tray!"));
            }
            Event::Menu(id) if id.0 == QUIT => {
                let _ = handle.quit();
            }
            _ => {}
        }
    });

    if let Err(err) = result {
        eprintln!("event loop ended with error: {err}");
    }
}
