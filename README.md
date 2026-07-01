# ldtray

[![CI](https://github.com/KarpelesLab/ldtray/actions/workflows/ci.yml/badge.svg)](https://github.com/KarpelesLab/ldtray/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/ldtray.svg)](https://crates.io/crates/ldtray)
[![docs.rs](https://img.shields.io/docsrs/ldtray)](https://docs.rs/ldtray)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Cross-platform tray icons for Rust that are **never linked against any GUI or
platform library at compile time**.

Every platform toolkit — libdbus on Linux, `shell32`/`user32` on Windows,
AppKit/`objc` on macOS — is resolved at *runtime* through
[`libloading`](https://docs.rs/libloading). The result: one daemon binary runs
everywhere. On a headless server the only failure is a clean `Err` from
`Tray::new` ("the tray library could not be loaded") — never a link error, never
a crash. Ignore the error and your program keeps running without a tray.

## Features

- Tray icon from raw RGBA pixels
- Click triggers: left / right / middle / double click
- Context menu with buttons, checkboxes, separators, and submenus
- Desktop notifications (with clickable action buttons on Linux)
- Update icon, tooltip, and menu live from any thread via a `Send + Sync` handle
- Blocking `run()` (main-thread-correct, works on macOS) **or** background
  `spawn()` (Linux/Windows)

## Status

All three backends implement the full feature set — tray icon, left/right/middle/
double-click triggers, context menu, and notifications:

| Platform | Mechanism                                | Validation                                    |
| -------- | ---------------------------------------- | --------------------------------------------- |
| Linux    | StatusNotifierItem + dbusmenu over D-Bus | end-to-end incl. clicks/menu/notify (KDE)     |
| Windows  | `Shell_NotifyIcon` + hidden window       | build + clippy + runtime smoke on CI          |
| macOS    | `NSStatusItem` via the Obj-C runtime     | build + clippy + runtime smoke on CI          |

Every backend is exercised at runtime by `examples/smoke.rs`, which CI runs on
the Windows and macOS GUI runners (and Linux): it creates the tray, pumps the
loop, updates icon/tooltip/menu, fires a notification and quits — so wrong FFI
struct layouts or message signatures fail the build. Synthetic click delivery
still needs a real interactive desktop; only the Linux triggers/menu are
verified against a live host end-to-end.

## Usage

```rust
use ldtray::{Tray, TrayConfig, Icon, Menu, MenuItem, Event, Notification};

let icon = Icon::from_rgba(1, 1, vec![255, 0, 0, 255])?;
let menu = Menu::new()
    .item(MenuItem::button(1, "Say hi"))
    .item(MenuItem::separator())
    .item(MenuItem::button(2, "Quit"));

let tray = match Tray::new(TrayConfig::new(icon).tooltip("demo").menu(menu)) {
    Ok(tray) => tray,
    Err(err) => {
        eprintln!("no tray available ({err}); continuing headless");
        return Ok(());
    }
};

let handle = tray.handle();
tray.run(move |event| match event {
    Event::Menu(id) if id.0 == 1 => { let _ = handle.notify(Notification::new("demo", "hi")); }
    Event::Menu(id) if id.0 == 2 => { let _ = handle.quit(); }
    other => println!("event: {other:?}"),
})?;
```

Run the bundled example:

```sh
cargo run --example basic
```

## Design

The whole crate compiles with a single non-platform dependency (`libloading`).
Platform symbols are hand-bound FFI declarations resolved from a `dlopen`'d
library the first time a `Tray` is created. See `src/platform/` for the
per-OS backends and `docs`/source comments for the wire-level details.

## License

MIT © Karpelès Lab Inc.
