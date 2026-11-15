//! The menus in macOS's menu bar, at the top of the screen: Massif's own
//! (About, Settings…, Services, Hide, Quit), then File, Edit, View and
//! Help, made from the list the window's top bar draws (`menu`) and kept in
//! step with it frame after frame. What is picked there waits here until
//! the app takes it, at its next frame, as files the Finder asks to open do
//! (`finder`).
//!
//! File, Edit, View and Help show their shortcuts and leave the keys to the
//! window. AppKit gives the menus the first go at a ⌘ key, before the
//! window: had they taken ⌘Z or ⌘S, the key would never reach the app's own
//! handling of it — which knows when a text field is typed in, when a
//! dialog has a ⌘Z of its own (Edit ▸ Ores), when the graph's undo comes
//! first — and an item greyed out would swallow its key. So the keys go
//! where they always went, and these menus only show them. The app menu's
//! keys are AppKit's, as in every Mac app: ⌘, (Settings), ⌘H, ⌥⌘H and ⌘Q.
//!
//! Elsewhere the menus stay in the window, and nothing waits here.

use crate::menu::{Command, Menu};
use std::cell::RefCell;
use std::sync::OnceLock;

/// Whether the system shows the menus in a bar of its own.
pub const NATIVE: bool = cfg!(target_os = "macos");

static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

thread_local! {
    /// What was picked. AppKit picks on the main thread, where the app's
    /// frames run and take it (and each test, on its own thread, has its
    /// own).
    static PICKED: RefCell<Vec<Command>> = const { RefCell::new(Vec::new()) };
}

/// From now on, show the menus in the system's menu bar (on macOS), and
/// wake the app when one of their items is picked. Called by the app once
/// its window is up — never by tests.
pub fn install(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// Show `menus` in the system's menu bar, once installed: nothing is done
/// while they are the ones shown already.
pub fn sync(menus: &[Menu]) {
    #[cfg(target_os = "macos")]
    if CONTEXT.get().is_some() {
        mac::sync(menus);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = menus;
}

/// What was picked in the system's menu bar since the last call.
pub fn take() -> Vec<Command> {
    PICKED.with(RefCell::take)
}

/// `command` picked in the system's menu bar: the app does it at its next
/// frame, if it can still be done then.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) fn pick(command: Command) {
    PICKED.with(|picked| picked.borrow_mut().push(command));
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use crate::menu::{self, Command, Entry, Item, Menu};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::{NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu, NSMenuItem};
    use objc2_foundation::{NSString, ns_string};
    use std::cell::RefCell;

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and `Target`
        // does not implement `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MassifMenuTarget"]
        struct Target;

        impl Target {
            /// An item picked: its tag is where its command is in
            /// `Bar::commands`.
            #[unsafe(method(pick:))]
            fn pick(&self, item: &NSMenuItem) {
                let tag = item.tag();
                let command = BAR.with(|bar| {
                    let bar = bar.try_borrow().ok()?;
                    bar.as_ref()?.commands.get(usize::try_from(tag).ok()?).cloned()
                });
                if let Some(command) = command {
                    super::pick(command);
                }
            }
        }
    );

    define_class!(
        // SAFETY: NSMenu may be subclassed, and `ShowOnly` does not
        // implement `Drop`.
        #[unsafe(super(NSMenu, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MassifShowOnlyMenu"]
        struct ShowOnly;

        impl ShowOnly {
            /// Its shortcuts are shown, never performed: the key goes on to
            /// the window, where the app handles it.
            #[unsafe(method(performKeyEquivalent:))]
            fn perform_key_equivalent(&self, _event: &AnyObject) -> bool {
                false
            }
        }
    );

    impl ShowOnly {
        fn new(mtm: MainThreadMarker, title: &str) -> Retained<Self> {
            // SAFETY: NSMenu's designated initialiser, which `ShowOnly`
            // keeps, with no instance variables of its own to set.
            let menu: Retained<Self> = unsafe { msg_send![Self::alloc(mtm), initWithTitle: &*NSString::from_str(title)] };
            // Greyed out or not as the list says, not as AppKit would guess.
            menu.setAutoenablesItems(false);
            menu
        }
    }

    /// The menu bar, as installed.
    struct Bar {
        target: Retained<Target>,
        main: Retained<NSMenu>,
        about: Retained<NSMenuItem>,
        settings: Retained<NSMenuItem>,
        quit: Retained<NSMenuItem>,
        /// File, Edit, View and Help.
        menus: Vec<Retained<ShowOnly>>,
        /// What each item picks, at its tag.
        commands: Vec<Command>,
        /// The list shown.
        shown: Vec<Menu>,
    }

    thread_local! {
        static BAR: RefCell<Option<Bar>> = const { RefCell::new(None) };
    }

    pub fn sync(menus: &[Menu]) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        BAR.with(|bar| {
            let Ok(mut bar) = bar.try_borrow_mut() else { return };
            let bar = bar.get_or_insert_with(|| Bar::install(mtm));
            if bar.shown.as_slice() != menus {
                bar.show(mtm, menus);
            }
        });
    }

    fn flags(modifiers: egui::Modifiers) -> NSEventModifierFlags {
        let mut flags = NSEventModifierFlags::empty();
        for (on, flag) in [
            (modifiers.command || modifiers.mac_cmd, NSEventModifierFlags::Command),
            (modifiers.shift, NSEventModifierFlags::Shift),
            (modifiers.alt, NSEventModifierFlags::Option),
            (modifiers.ctrl, NSEventModifierFlags::Control),
        ] {
            if on {
                flags |= flag;
            }
        }
        flags
    }

    impl Bar {
        /// Massif's menu bar, in place of winit's (its app menu alone).
        fn install(mtm: MainThreadMarker) -> Self {
            let app = NSApplication::sharedApplication(mtm);
            // SAFETY: NSObject's initialiser; `Target` has no instance
            // variables to set.
            let target: Retained<Target> = unsafe { msg_send![Target::alloc(mtm), init] };
            let system = |title: &NSString, action, key: &NSString, modifiers| {
                // SAFETY: the actions are NSApplication's, which answers
                // them at the end of the responder chain (no target).
                let item = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), title, Some(action), key) };
                item.setKeyEquivalentModifierMask(modifiers);
                item
            };
            let (about, settings, quit) = (NSMenuItem::new(mtm), NSMenuItem::new(mtm), NSMenuItem::new(mtm));
            let services = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Services"));
            let services_item = NSMenuItem::new(mtm);
            services_item.setTitle(ns_string!("Services"));
            services_item.setSubmenu(Some(&services));
            let app_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Massif"));
            app_menu.setAutoenablesItems(false);
            let separator = || NSMenuItem::separatorItem(mtm);
            for item in [
                about.clone(),
                separator(),
                settings.clone(),
                separator(),
                services_item,
                separator(),
                system(ns_string!("Hide Massif"), sel!(hide:), ns_string!("h"), NSEventModifierFlags::Command),
                system(ns_string!("Hide Others"), sel!(hideOtherApplications:), ns_string!("h"), NSEventModifierFlags::Command | NSEventModifierFlags::Option),
                system(ns_string!("Show All"), sel!(unhideAllApplications:), ns_string!(""), NSEventModifierFlags::empty()),
                separator(),
                quit.clone(),
            ] {
                app_menu.addItem(&item);
            }
            let main = NSMenu::new(mtm);
            let app_item = NSMenuItem::new(mtm);
            app_item.setSubmenu(Some(&app_menu));
            main.addItem(&app_item);
            app.setMainMenu(Some(&main));
            app.setServicesMenu(Some(&services));
            Bar {
                target,
                main,
                about,
                settings,
                quit,
                menus: Vec::new(),
                commands: Vec::new(),
                shown: Vec::new(),
            }
        }

        /// Show `menus`: About, Settings… and Quit in the app menu, the rest
        /// in File, Edit, View and Help, made again.
        fn show(&mut self, mtm: MainThreadMarker, menus: &[Menu]) {
            let mac = menu::for_macos(menus);
            self.commands.clear();
            for (native, item) in [(self.about.clone(), &mac.about), (self.settings.clone(), &mac.settings), (self.quit.clone(), &mac.quit)] {
                native.setHidden(item.is_none());
                if let Some(item) = item {
                    self.set(&native, item);
                }
            }
            // As Mac apps name it, with its key.
            self.quit.setTitle(ns_string!("Quit Massif"));
            self.quit.setKeyEquivalent(ns_string!("q"));
            self.quit.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
            let titles: Vec<String> = self.menus.iter().map(|m| m.title().to_string()).collect();
            if titles.iter().map(String::as_str).ne(mac.menus.iter().map(|m| m.title)) {
                while self.main.numberOfItems() > 1 {
                    self.main.removeItemAtIndex(1);
                }
                self.menus.clear();
                for menu in &mac.menus {
                    let native = ShowOnly::new(mtm, menu.title);
                    let item = NSMenuItem::new(mtm);
                    item.setTitle(&NSString::from_str(menu.title));
                    item.setSubmenu(Some(&native));
                    self.main.addItem(&item);
                    self.menus.push(native);
                }
                // The Help menu gets the system's search of the menus.
                let help: Option<&NSMenu> = self.menus.iter().find(|m| m.title().to_string() == "Help").map(|m| -> &NSMenu { m });
                NSApplication::sharedApplication(mtm).setHelpMenu(help);
            }
            for (native, menu) in self.menus.clone().iter().zip(&mac.menus) {
                native.removeAllItems();
                self.fill(mtm, native, &menu.entries);
            }
            self.shown = menus.to_vec();
        }

        fn fill(&mut self, mtm: MainThreadMarker, native: &NSMenu, entries: &[Entry]) {
            for entry in entries {
                match entry {
                    Entry::Separator => native.addItem(&NSMenuItem::separatorItem(mtm)),
                    Entry::Item(item) => {
                        let it = NSMenuItem::new(mtm);
                        self.set(&it, item);
                        native.addItem(&it);
                    }
                    Entry::Submenu { label, enabled, entries, .. } => {
                        let sub = ShowOnly::new(mtm, label);
                        self.fill(mtm, &sub, entries);
                        let it = NSMenuItem::new(mtm);
                        it.setTitle(&NSString::from_str(label));
                        it.setSubmenu(Some(&sub));
                        it.setEnabled(*enabled);
                        native.addItem(&it);
                    }
                }
            }
        }

        /// `native` as `item` has it: its name, whether it can be picked,
        /// its tick, its tip, its shortcut, and what it picks.
        fn set(&mut self, native: &NSMenuItem, item: &Item) {
            native.setTitle(&NSString::from_str(&item.label));
            native.setEnabled(item.enabled);
            native.setState(if item.checked == Some(true) { NSControlStateValueOn } else { NSControlStateValueOff });
            native.setToolTip(item.tip.as_deref().map(NSString::from_str).as_deref());
            let (key, modifiers) = item.command.shortcut().and_then(|s| menu::key_equivalent(&s)).unwrap_or_default();
            native.setKeyEquivalent(&NSString::from_str(&key));
            native.setKeyEquivalentModifierMask(flags(modifiers));
            let target: &AnyObject = &self.target;
            // SAFETY: `Target` answers `pick:`, and the bar keeps it alive
            // as long as its items.
            unsafe {
                native.setTarget(Some(target));
                native.setAction(Some(sel!(pick:)));
            }
            native.setTag(self.commands.len() as isize);
            self.commands.push(item.command.clone());
        }
    }
}
