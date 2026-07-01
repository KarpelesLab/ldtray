//! macOS tray backend: an `NSStatusItem` driven through the Objective-C runtime.
//!
//! AppKit and `libobjc` are loaded at runtime (see [`objc`]); the crate links
//! against neither. The status item, its button, image and menu are created on
//! the first `pump` so they live on the thread running the loop — AppKit UI must
//! be used from the main thread, which is why [`Backend::can_spawn`] is `false`
//! here (use [`crate::Tray::run`] from `main`). Button clicks are delivered to a
//! dynamically-registered delegate class and become [`Event`]s; the context menu
//! pops up on right/ctrl-click.

mod objc;

use std::os::raw::c_void;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;
use std::time::Duration;

use objc::*;

use super::{Backend, Init};
use crate::error::Result;
use crate::event::Event;
use crate::icon::Icon;
use crate::menu::{Menu, MenuId, MenuItem};
use crate::notification::{ActionId, Notification};

/// `NSApplicationActivationPolicyAccessory` — menu-bar UI, no Dock icon.
const ACTIVATION_ACCESSORY: i64 = 1;
/// `NSVariableStatusItemLength`.
const VARIABLE_LENGTH: f64 = -1.0;
/// `NSEventModifierFlagControl`.
const MOD_CONTROL: u64 = 1 << 18;
// NSEventType values.
const EVENT_LEFT_UP: u64 = 2;
const EVENT_RIGHT_UP: u64 = 4;
const EVENT_OTHER_UP: u64 = 26;
/// `sendActionOn:` mask covering left/right/other mouse-up.
const ACTION_MASK: u64 = (1 << 2) | (1 << 4) | (1 << 26);
// NSUserNotificationActivationType values.
const ACTIVATION_ACTION_BUTTON: i64 = 2;
const ACTIVATION_ADDITIONAL_ACTION: i64 = 4;

static OBJC: OnceLock<ObjC> = OnceLock::new();
static DELEGATE_CLASS: OnceLock<usize> = OnceLock::new();

fn ensure_objc() -> Result<&'static ObjC> {
    if let Some(objc) = OBJC.get() {
        return Ok(objc);
    }
    let loaded = ObjC::load()?;
    let _ = OBJC.set(loaded);
    Ok(OBJC.get().expect("objc set"))
}

fn objc() -> &'static ObjC {
    OBJC.get().expect("objc initialized before use")
}

pub(crate) fn new(init: Init) -> Result<Box<dyn Backend>> {
    ensure_objc()?; // surfaces LibraryLoad on a headless box
    Ok(Box::new(MacBackend::new(init)))
}

struct State {
    app: id,
    item: id,
    button: id,
    menu_obj: id,
    delegate: id,
    mode: id,
    icon: (i64, i64, Vec<u8>),
    tooltip: String,
    menu: Option<Menu>,
    pending: Vec<Event>,
    started: bool,
}

// Only ever touched from the main/loop thread (can_spawn == false).
unsafe impl Send for State {}

pub(crate) struct MacBackend {
    state: Box<State>,
}

impl MacBackend {
    fn new(init: Init) -> MacBackend {
        let state = Box::new(State {
            app: std::ptr::null_mut(),
            item: std::ptr::null_mut(),
            button: std::ptr::null_mut(),
            menu_obj: std::ptr::null_mut(),
            delegate: std::ptr::null_mut(),
            mode: std::ptr::null_mut(),
            icon: (
                init.icon.width as i64,
                init.icon.height as i64,
                init.icon.rgba.clone(),
            ),
            tooltip: init.tooltip,
            menu: init.menu,
            pending: Vec::new(),
            started: false,
        });
        MacBackend { state }
    }

    fn ensure_started(&mut self) {
        if !self.state.started {
            unsafe { self.state.start() };
            self.state.started = true;
        }
    }
}

impl State {
    unsafe fn start(&mut self) {
        let objc = objc();
        unsafe {
            let app = objc.send0(objc.class(c"NSApplication"), objc.sel(c"sharedApplication"));
            objc.send_void_i64(app, objc.sel(c"setActivationPolicy:"), ACTIVATION_ACCESSORY);
            objc.send0(app, objc.sel(c"finishLaunching"));
            self.app = app;

            let dclass = delegate_class(objc);
            let delegate = objc.send0(objc.send0(dclass, objc.sel(c"alloc")), objc.sel(c"init"));
            objc.set_ivar(delegate, c"ldState", self as *mut State as *mut c_void);
            self.delegate = delegate;

            let bar = objc.send0(objc.class(c"NSStatusBar"), objc.sel(c"systemStatusBar"));
            let item = objc.send_f64(bar, objc.sel(c"statusItemWithLength:"), VARIABLE_LENGTH);
            objc.send0(item, objc.sel(c"retain"));
            self.item = item;
            self.button = objc.send0(item, objc.sel(c"button"));

            self.apply_image(objc);
            self.apply_tooltip(objc);

            objc.send_void_id(self.button, objc.sel(c"setTarget:"), delegate);
            objc.send_void_sel(self.button, objc.sel(c"setAction:"), objc.sel(c"onClick:"));
            objc.send_void_u64(self.button, objc.sel(c"sendActionOn:"), ACTION_MASK);

            self.menu_obj = self.build_menu(objc);

            let mode = objc.nsstring("kCFRunLoopDefaultMode");
            objc.send0(mode, objc.sel(c"retain"));
            self.mode = mode;
        }
    }

    unsafe fn apply_image(&self, objc: &ObjC) {
        if self.button.is_null() {
            return;
        }
        unsafe {
            let image = make_image(objc, self.icon.0, self.icon.1, &self.icon.2);
            if !image.is_null() {
                objc.send_void_id(self.button, objc.sel(c"setImage:"), image);
            }
        }
    }

    unsafe fn apply_tooltip(&self, objc: &ObjC) {
        if self.button.is_null() {
            return;
        }
        unsafe {
            let s = objc.nsstring(&self.tooltip);
            objc.send_void_id(self.button, objc.sel(c"setToolTip:"), s);
        }
    }

    /// Builds a retained `NSMenu` from the current menu, or null if none.
    unsafe fn build_menu(&self, objc: &ObjC) -> id {
        match &self.menu {
            None => std::ptr::null_mut(),
            Some(menu) => unsafe {
                let ns = self.build_nsmenu(objc, menu.items());
                objc.send0(ns, objc.sel(c"retain"));
                ns
            },
        }
    }

    unsafe fn build_nsmenu(&self, objc: &ObjC, items: &[MenuItem]) -> id {
        unsafe {
            let menu = objc.send0(
                objc.send0(objc.class(c"NSMenu"), objc.sel(c"alloc")),
                objc.sel(c"init"),
            );
            objc.send_void_i8(menu, objc.sel(c"setAutoenablesItems:"), 0);
            let empty = objc.nsstring("");
            for item in items {
                match item {
                    MenuItem::Separator => {
                        let sep = objc.send0(objc.class(c"NSMenuItem"), objc.sel(c"separatorItem"));
                        objc.send_void_id(menu, objc.sel(c"addItem:"), sep);
                    }
                    MenuItem::Button {
                        id: menu_id,
                        label,
                        enabled,
                        checked,
                    } => {
                        let title = objc.nsstring(label);
                        let mi = objc.send_init_item(
                            objc.send0(objc.class(c"NSMenuItem"), objc.sel(c"alloc")),
                            objc.sel(c"initWithTitle:action:keyEquivalent:"),
                            title,
                            objc.sel(c"onMenu:"),
                            empty,
                        );
                        objc.send_void_id(mi, objc.sel(c"setTarget:"), self.delegate);
                        objc.send_void_i64(mi, objc.sel(c"setTag:"), menu_id.0 as i64);
                        if let Some(checked) = checked {
                            objc.send_void_i64(
                                mi,
                                objc.sel(c"setState:"),
                                if *checked { 1 } else { 0 },
                            );
                        }
                        if !enabled {
                            objc.send_void_i8(mi, objc.sel(c"setEnabled:"), 0);
                        }
                        objc.send_void_id(menu, objc.sel(c"addItem:"), mi);
                    }
                    MenuItem::Submenu {
                        label,
                        enabled,
                        items,
                    } => {
                        let title = objc.nsstring(label);
                        let mi = objc.send_init_item(
                            objc.send0(objc.class(c"NSMenuItem"), objc.sel(c"alloc")),
                            objc.sel(c"initWithTitle:action:keyEquivalent:"),
                            title,
                            std::ptr::null_mut(),
                            empty,
                        );
                        let sub = self.build_nsmenu(objc, items);
                        objc.send_void_id(mi, objc.sel(c"setSubmenu:"), sub);
                        if !enabled {
                            objc.send_void_i8(mi, objc.sel(c"setEnabled:"), 0);
                        }
                        objc.send_void_id(menu, objc.sel(c"addItem:"), mi);
                    }
                }
            }
            menu
        }
    }

    /// Pops up the status item's context menu (used on right/ctrl-click).
    unsafe fn popup(&self, objc: &ObjC) {
        if !self.menu_obj.is_null() {
            unsafe {
                objc.send_void_id(self.item, objc.sel(c"popUpStatusItemMenu:"), self.menu_obj);
            }
        }
    }

    unsafe fn show_notification(&self, objc: &ObjC, notification: &Notification) {
        unsafe {
            let cls = objc.class(c"NSUserNotification");
            if cls.is_null() {
                return;
            }
            let note = objc.send0(objc.send0(cls, objc.sel(c"alloc")), objc.sel(c"init"));
            objc.send_void_id(
                note,
                objc.sel(c"setTitle:"),
                objc.nsstring(&notification.title),
            );
            objc.send_void_id(
                note,
                objc.sel(c"setInformativeText:"),
                objc.nsstring(&notification.body),
            );

            // Actions: the first becomes the primary action button (its id is
            // stashed in the notification's identifier); the rest become
            // additionalActions, each carrying its ActionId as the identifier.
            let mut actions = notification.actions.iter();
            if let Some((first_id, first_label)) = actions.next() {
                objc.send_void_id(
                    note,
                    objc.sel(c"setIdentifier:"),
                    objc.nsstring(&first_id.0.to_string()),
                );
                objc.send_void_i8(note, objc.sel(c"setHasActionButton:"), 1);
                objc.send_void_id(
                    note,
                    objc.sel(c"setActionButtonTitle:"),
                    objc.nsstring(first_label),
                );

                let action_cls = objc.class(c"NSUserNotificationAction");
                if !action_cls.is_null() {
                    let array = objc.send0(objc.class(c"NSMutableArray"), objc.sel(c"array"));
                    for (id, label) in actions {
                        let action = objc.send_id_id(
                            action_cls,
                            objc.sel(c"actionWithIdentifier:title:"),
                            objc.nsstring(&id.0.to_string()),
                            objc.nsstring(label),
                        );
                        objc.send_void_id(array, objc.sel(c"addObject:"), action);
                    }
                    objc.send_void_id(note, objc.sel(c"setAdditionalActions:"), array);
                }
            }

            let center = objc.send0(
                objc.class(c"NSUserNotificationCenter"),
                objc.sel(c"defaultUserNotificationCenter"),
            );
            if !center.is_null() {
                // Route action clicks back to us via our delegate.
                objc.send_void_id(center, objc.sel(c"setDelegate:"), self.delegate);
                objc.send_void_id(center, objc.sel(c"deliverNotification:"), note);
            }
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if !self.started {
            return;
        }
        let objc = objc();
        unsafe {
            let bar = objc.send0(objc.class(c"NSStatusBar"), objc.sel(c"systemStatusBar"));
            if !self.item.is_null() {
                objc.send_void_id(bar, objc.sel(c"removeStatusItem:"), self.item);
                objc.send0(self.item, objc.sel(c"release"));
            }
            for obj in [self.menu_obj, self.delegate, self.mode] {
                if !obj.is_null() {
                    objc.send0(obj, objc.sel(c"release"));
                }
            }
        }
    }
}

impl Backend for MacBackend {
    fn set_icon(&mut self, icon: &Icon) -> Result<()> {
        self.state.icon = (icon.width as i64, icon.height as i64, icon.rgba.clone());
        if self.state.started {
            unsafe { self.state.apply_image(objc()) };
        }
        Ok(())
    }

    fn set_tooltip(&mut self, text: &str) -> Result<()> {
        self.state.tooltip = text.to_string();
        if self.state.started {
            unsafe { self.state.apply_tooltip(objc()) };
        }
        Ok(())
    }

    fn set_menu(&mut self, menu: Option<&Menu>) -> Result<()> {
        self.state.menu = menu.cloned();
        if self.state.started {
            let objc = objc();
            unsafe {
                if !self.state.menu_obj.is_null() {
                    objc.send0(self.state.menu_obj, objc.sel(c"release"));
                    self.state.menu_obj = std::ptr::null_mut();
                }
                self.state.menu_obj = self.state.build_menu(objc);
            }
        }
        Ok(())
    }

    fn notify(&mut self, notification: &Notification) -> Result<()> {
        self.ensure_started();
        unsafe { self.state.show_notification(objc(), notification) };
        Ok(())
    }

    fn pump(&mut self, timeout: Duration, sink: &mut dyn FnMut(Event)) -> Result<()> {
        self.ensure_started();
        // Copy handles out so no borrow of `state` is held while a dispatched
        // event re-enters the delegate (which forms its own &mut State).
        let objc = objc();
        let app = self.state.app;
        let mode = self.state.mode;
        let seconds = timeout.as_secs_f64();
        unsafe {
            let sel_next = objc.sel(c"nextEventMatchingMask:untilDate:inMode:dequeue:");
            let sel_send = objc.sel(c"sendEvent:");
            let pool = objc.send0(
                objc.send0(objc.class(c"NSAutoreleasePool"), objc.sel(c"alloc")),
                objc.sel(c"init"),
            );
            let mut first = true;
            loop {
                let date = if first {
                    objc.send_f64(
                        objc.class(c"NSDate"),
                        objc.sel(c"dateWithTimeIntervalSinceNow:"),
                        seconds,
                    )
                } else {
                    objc.send0(objc.class(c"NSDate"), objc.sel(c"distantPast"))
                };
                let event = objc.send_next_event(app, sel_next, u64::MAX, date, mode, 1);
                if event.is_null() {
                    break;
                }
                objc.send_void_id(app, sel_send, event);
                first = false;
            }
            objc.send0(pool, objc.sel(c"drain"));
        }
        for event in self.state.pending.drain(..) {
            sink(event);
        }
        Ok(())
    }

    fn can_spawn(&self) -> bool {
        false
    }
}

/// Registers the `LDTrayDelegate` Objective-C class once and returns it.
unsafe fn delegate_class(objc: &ObjC) -> Class {
    let cls = *DELEGATE_CLASS.get_or_init(|| unsafe {
        let superclass = objc.class(c"NSObject");
        let cls = objc.allocate_class(superclass, c"LDTrayDelegate");
        // A void* ivar holding the Rust State pointer.
        objc.add_ivar(cls, c"ldState", 8, 3, c"^v");
        let on_click: unsafe extern "C" fn(id, SEL, id) = delegate_on_click;
        let on_menu: unsafe extern "C" fn(id, SEL, id) = delegate_on_menu;
        let on_notify: unsafe extern "C" fn(id, SEL, id, id) = delegate_on_notification;
        objc.add_method(
            cls,
            objc.sel(c"onClick:"),
            std::mem::transmute::<unsafe extern "C" fn(id, SEL, id), Imp>(on_click),
            c"v@:@",
        );
        objc.add_method(
            cls,
            objc.sel(c"onMenu:"),
            std::mem::transmute::<unsafe extern "C" fn(id, SEL, id), Imp>(on_menu),
            c"v@:@",
        );
        // NSUserNotificationCenterDelegate: userNotificationCenter:didActivateNotification:
        objc.add_method(
            cls,
            objc.sel(c"userNotificationCenter:didActivateNotification:"),
            std::mem::transmute::<unsafe extern "C" fn(id, SEL, id, id), Imp>(on_notify),
            c"v@:@@",
        );
        objc.register_class(cls);
        cls as usize
    });
    cls as Class
}

unsafe fn state_from(this: id) -> *mut State {
    unsafe { objc().get_ivar(this, c"ldState") as *mut State }
}

/// Button action: turn the current mouse event into a click [`Event`].
unsafe extern "C" fn delegate_on_click(this: id, _cmd: SEL, _sender: id) {
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        let objc = objc();
        let ptr = state_from(this);
        if ptr.is_null() {
            return;
        }
        let state = &mut *ptr;
        let event = objc.send0(state.app, objc.sel(c"currentEvent"));
        if event.is_null() {
            return;
        }
        let kind = objc.send_ret_u64(event, objc.sel(c"type"));
        let mods = objc.send_ret_u64(event, objc.sel(c"modifierFlags"));
        let ctrl = mods & MOD_CONTROL != 0;
        match kind {
            EVENT_LEFT_UP => {
                if ctrl {
                    state.pending.push(Event::RightClick);
                    state.popup(objc);
                } else {
                    let clicks = objc.send_ret_i64(event, objc.sel(c"clickCount"));
                    state.pending.push(if clicks >= 2 {
                        Event::DoubleClick
                    } else {
                        Event::LeftClick
                    });
                }
            }
            EVENT_RIGHT_UP => {
                state.pending.push(Event::RightClick);
                state.popup(objc);
            }
            EVENT_OTHER_UP => state.pending.push(Event::MiddleClick),
            _ => {}
        }
    }));
}

/// Menu action: map the selected item's tag back to its [`MenuId`].
unsafe extern "C" fn delegate_on_menu(this: id, _cmd: SEL, sender: id) {
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        let objc = objc();
        let ptr = state_from(this);
        if ptr.is_null() {
            return;
        }
        let tag = objc.send_ret_i64(sender, objc.sel(c"tag"));
        (*ptr).pending.push(Event::Menu(MenuId(tag as u32)));
    }));
}

/// `NSUserNotificationCenterDelegate`: a notification action was clicked. The
/// action button carries its id in the notification's `identifier`; an
/// additional action carries it in that action's `identifier`.
unsafe extern "C" fn delegate_on_notification(this: id, _cmd: SEL, _center: id, notification: id) {
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        let objc = objc();
        let ptr = state_from(this);
        if ptr.is_null() {
            return;
        }
        let kind = objc.send_ret_i64(notification, objc.sel(c"activationType"));
        let identifier = match kind {
            ACTIVATION_ACTION_BUTTON => {
                objc.nsstring_to_rust(objc.send0(notification, objc.sel(c"identifier")))
            }
            ACTIVATION_ADDITIONAL_ACTION => {
                let action = objc.send0(notification, objc.sel(c"additionalActivationAction"));
                objc.nsstring_to_rust(objc.send0(action, objc.sel(c"identifier")))
            }
            _ => None,
        };
        if let Some(id) = identifier.and_then(|s| s.parse::<u32>().ok()) {
            (*ptr).pending.push(Event::NotificationAction(ActionId(id)));
        }
    }));
}

/// Builds an `NSImage` from RGBA pixels via an `NSBitmapImageRep` whose buffer
/// AppKit allocates and we then fill (so no lifetime coupling to `rgba`).
unsafe fn make_image(objc: &ObjC, width: i64, height: i64, rgba: &[u8]) -> id {
    unsafe {
        let colorspace = objc.nsstring("NSDeviceRGBColorSpace");
        let rep_alloc = objc.send0(objc.class(c"NSBitmapImageRep"), objc.sel(c"alloc"));
        let mut planes: *mut u8 = std::ptr::null_mut();
        let rep = objc.send_init_bitmap(
            rep_alloc,
            objc.sel(c"initWithBitmapDataPlanes:pixelsWide:pixelsHigh:bitsPerSample:samplesPerPixel:hasAlpha:isPlanar:colorSpaceName:bytesPerRow:bitsPerPixel:"),
            &mut planes,
            width,
            height,
            8,
            4,
            1,
            0,
            colorspace,
            width * 4,
            32,
        );
        if rep.is_null() {
            return std::ptr::null_mut();
        }
        let data = objc.send0(rep, objc.sel(c"bitmapData")) as *mut u8;
        if !data.is_null() {
            let n = ((width * height * 4) as usize).min(rgba.len());
            std::ptr::copy_nonoverlapping(rgba.as_ptr(), data, n);
        }
        let image = objc.send_size(
            objc.send0(objc.class(c"NSImage"), objc.sel(c"alloc")),
            objc.sel(c"initWithSize:"),
            NSSize {
                width: width as f64,
                height: height as f64,
            },
        );
        objc.send_void_id(image, objc.sel(c"addRepresentation:"), rep);
        objc.send_void_i8(image, objc.sel(c"setTemplate:"), 0);
        image
    }
}
