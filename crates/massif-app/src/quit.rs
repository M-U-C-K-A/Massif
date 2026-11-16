//! The system's Quit, on macOS: the Dock's (right-click ▸ Quit), the app
//! switcher's (⌘Tab, then Q), a script's, and the one sent at a logout, a
//! restart or a shut-down.
//!
//! AppKit asks the application's delegate first, as
//! `applicationShouldTerminate:`, which winit's delegate does not answer: the
//! app was ended at once, and the map's changes were lost. So every object is
//! taught to answer it — only the delegate is ever asked — as `finder` does
//! for the files the Finder opens, and the answer comes from what the app
//! said at the end of its last frame:
//!
//! - nothing to lose: quit now, as before;
//! - changes that are not saved: not now. Massif comes to the front and, at
//!   its next frame, asks as its own Quit does (Unsaved), then quits by
//!   itself if it is told to (Don't Save, or once the map is saved);
//! - the same at a logout, a restart or a shut-down, which wait for the
//!   answer (two minutes at most): later. It is given once the dialog is
//!   answered — yes, Massif ends and the logout goes on; no (Cancel, or the
//!   save did not happen), Massif stays and the logout stops.
//!
//! While a frame runs (a file dialog is open in it), the answer is not now,
//! whatever there is to lose: winit cannot end the app in the middle of a
//! frame, and AppKit's wait for a later answer would never end, as the frames
//! that give it wait for the dialog. The app takes the question at its next
//! frame.
//!
//! On other systems nothing asks here.

use std::cell::Cell;
use std::sync::OnceLock;

static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

thread_local! {
    // AppKit asks on the main thread, where the app's frames run and tell
    // (and each test, on its own thread, has its own).
    /// The map has changes the app would ask about before quitting, as its
    /// last frame left it.
    static UNSAVED: Cell<bool> = const { Cell::new(false) };
    /// One of the app's frames is running.
    static IN_FRAME: Cell<bool> = const { Cell::new(false) };
    /// The system asked to quit, and the app has not taken it yet.
    static ASKED: Cell<bool> = const { Cell::new(false) };
    /// The system waits for the answer: it was told later.
    static WAITING: Cell<bool> = const { Cell::new(false) };
    /// The last answer given to a wait.
    #[cfg(test)]
    static REPLIED: Cell<Option<bool>> = const { Cell::new(None) };
}

/// The answers to AppKit's question, its NSApplicationTerminateReply.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// Not now (NSTerminateCancel): the app asks first, as its own Quit
    /// does, and quits by itself if it is told to.
    Cancel = 0,
    /// Quit now (NSTerminateNow): nothing would be lost.
    Now = 1,
    /// Later (NSTerminateLater): the system waits for the answer, given once
    /// the app's question is answered.
    Later = 2,
}

/// The answer to the system's quit. `unsaved`: the map has changes the app
/// would ask about; `in_frame`: one of the app's frames is running; `waits`:
/// the system waits for the answer (a logout, a restart, a shut-down);
/// `waiting`: it waits for an earlier one already.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub fn answer(unsaved: bool, in_frame: bool, waits: bool, waiting: bool) -> Reply {
    if !unsaved && !in_frame {
        Reply::Now
    } else if unsaved && waits && !in_frame && !waiting {
        Reply::Later
    } else {
        Reply::Cancel
    }
}

/// The system asks whether the app may quit (`waits`: and waits for the
/// answer). Answered from what the app said at the end of its last frame;
/// asked of the app at its next one, unless it may quit now.
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub fn ask(waits: bool) -> Reply {
    let reply = answer(UNSAVED.get(), IN_FRAME.get(), waits, WAITING.get());
    if reply == Reply::Later {
        WAITING.set(true);
    }
    if reply != Reply::Now {
        ASKED.set(true);
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }
    reply
}

/// From now on, answer AppKit, and wake the app when the system asks it to
/// quit. Called by the app once its window is up — never by tests.
pub fn install(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// One of the app's frames begins.
pub fn frame_begins() {
    IN_FRAME.set(true);
}

/// The frame ends, and with it what the system's quit is answered until the
/// next one: `unsaved`, whether the map has changes the app would ask about
/// before quitting.
pub fn frame_ends(unsaved: bool) {
    IN_FRAME.set(false);
    UNSAVED.set(unsaved);
}

/// Whether the system asked to quit since the last call.
pub fn take() -> bool {
    ASKED.replace(false)
}

/// Whether the system waits to know whether the app quits.
pub fn waiting() -> bool {
    WAITING.get()
}

/// Tell the system that waits whether the app quits: yes, and it ends the
/// app; no, and the app stays.
pub fn reply(quit: bool) {
    if !WAITING.replace(false) {
        return;
    }
    #[cfg(test)]
    REPLIED.set(Some(quit));
    #[cfg(target_os = "macos")]
    if CONTEXT.get().is_some() {
        mac::reply(quit);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = quit;
}

/// The answer given to the last wait since the last call.
#[cfg(test)]
pub fn replied() -> Option<bool> {
    REPLIED.take()
}

/// Start answering, before the application runs: AppKit reads what its
/// delegate answers once, when the delegate is set.
#[cfg(target_os = "macos")]
pub fn listen() {
    mac::listen();
}

#[cfg(not(target_os = "macos"))]
pub fn listen() {}

#[cfg(target_os = "macos")]
mod mac {
    use super::Reply;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Imp, NSObject, Sel};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSArray, NSObjectNSDelayedPerforming, NSRunLoopCommonModes};

    /// A four-character code of the Apple Events.
    const fn code(c: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*c)
    }

    /// What a quit says it is sent for (kAEQuitReason) when the system waits
    /// for it: every app quitting, a shut-down, a restart, a logout.
    const WAITS: [u32; 5] = [code(b"quia"), code(b"shut"), code(b"rest"), code(b"rlgo"), code(b"logo")];

    /// Whether the quit being handled is the system's at a logout, a restart
    /// or a shut-down, which waits for the answer: its Apple Event says why
    /// it is sent, where the Dock's, the app switcher's and a script's don't.
    fn waits() -> bool {
        let Some(event) = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent() else { return false };
        // SAFETY: an Apple Event's class and ID, and its descriptors by
        // keyword, all four-character codes (objc2 has them with Core
        // Services alone).
        let (class, id): (u32, u32) = unsafe { (msg_send![&*event, eventClass], msg_send![&*event, eventID]) };
        if class != code(b"aevt") || id != code(b"quit") {
            return false;
        }
        let why = code(b"why?");
        let attribute: Option<Retained<NSAppleEventDescriptor>> = unsafe { msg_send![&*event, attributeDescriptorForKeyword: why] };
        let Some(reason) = attribute.or_else(|| unsafe { msg_send![&*event, paramDescriptorForKeyword: why] }) else { return false };
        // A type code or an enumerated one, as it comes.
        let kind: u32 = unsafe { msg_send![&*reason, descriptorType] };
        WAITS.contains(&if kind == code(b"enum") { reason.enumCodeValue() } else { reason.typeCodeValue() })
    }

    /// Massif in front, its windows out of the Dock, for its question to be
    /// seen.
    fn to_front(mtm: MainThreadMarker) {
        let app = NSApplication::sharedApplication(mtm);
        app.unhide(None);
        // SAFETY: NSApplication's windows are NSWindows, which answer
        // isMiniaturized and deminiaturize:.
        let windows: Retained<NSArray<AnyObject>> = unsafe { msg_send![&*app, windows] };
        for window in windows.iter() {
            let minimised: bool = unsafe { msg_send![&*window, isMiniaturized] };
            if minimised {
                let _: () = unsafe { msg_send![&*window, deminiaturize: None::<&AnyObject>] };
            }
        }
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }

    unsafe extern "C-unwind" fn should_terminate(_this: *mut AnyObject, _cmd: Sel, _app: *mut AnyObject) -> usize {
        let reply = super::ask(waits());
        if reply != Reply::Now
            && let Some(mtm) = MainThreadMarker::new()
        {
            to_front(mtm);
        }
        reply as usize
    }

    pub fn listen() {
        let Some(object) = AnyClass::get(c"NSObject") else { return };
        // SAFETY: the method has the signature of `applicationShouldTerminate:`
        // (an NSUInteger from self, _cmd and an object), as its type string
        // says.
        unsafe {
            let imp: Imp = std::mem::transmute(should_terminate as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize);
            objc2::ffi::class_addMethod(object as *const AnyClass as *mut AnyClass, sel!(applicationShouldTerminate:), imp, c"Q@:@".as_ptr());
        }
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and `Answer` does
        // not implement `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MassifQuitAnswer"]
        struct Answer;

        impl Answer {
            #[unsafe(method(yes:))]
            fn yes(&self, _sender: Option<&AnyObject>) {
                NSApplication::sharedApplication(self.mtm()).replyToApplicationShouldTerminate(true);
            }

            #[unsafe(method(no:))]
            fn no(&self, _sender: Option<&AnyObject>) {
                NSApplication::sharedApplication(self.mtm()).replyToApplicationShouldTerminate(false);
            }
        }
    );

    /// Answer AppKit, which waits, once the frame that decided it is over:
    /// told yes, it ends the app, which winit can only do between frames.
    pub fn reply(quit: bool) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        // SAFETY: NSObject's initialiser; `Answer` has no instance variables
        // to set.
        let answer: Retained<Answer> = unsafe { msg_send![Answer::alloc(mtm), init] };
        // In every mode the run loop runs in common, AppKit's wait among them.
        // SAFETY: a constant of Foundation's.
        let modes = NSArray::from_slice(&[unsafe { NSRunLoopCommonModes }]);
        // SAFETY: `Answer` answers yes: and no:, which take an object (none
        // is given here); the run loop keeps it until then.
        unsafe { answer.performSelector_withObject_afterDelay_inModes(if quit { sel!(yes:) } else { sel!(no:) }, None, 0.0, &modes) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Now when there is nothing to lose; later when the system waits for
    /// the answer to a question about changes; not now otherwise — and
    /// always while a frame runs.
    #[test]
    fn the_answers() {
        for unsaved in [false, true] {
            for in_frame in [false, true] {
                for waits in [false, true] {
                    for waiting in [false, true] {
                        let expected = match (unsaved, in_frame) {
                            (false, false) => Reply::Now,
                            (true, false) if waits && !waiting => Reply::Later,
                            _ => Reply::Cancel,
                        };
                        assert_eq!(answer(unsaved, in_frame, waits, waiting), expected, "unsaved {unsaved}, in a frame {in_frame}, waits {waits}, waiting {waiting}");
                    }
                }
            }
        }
        // AppKit's values.
        assert_eq!([Reply::Cancel as usize, Reply::Now as usize, Reply::Later as usize], [0, 1, 2]);
    }

    /// A question asked but not taken is asked once; a wait is answered
    /// once, and an answer with nothing waiting is not given.
    #[test]
    fn asked_once_answered_once() {
        frame_ends(true);
        assert_eq!(ask(false), Reply::Cancel);
        assert!(take());
        assert!(!take(), "taken");
        assert!(!waiting());
        reply(true);
        assert_eq!(replied(), None, "nothing waited");
        assert_eq!(ask(true), Reply::Later);
        assert!(waiting());
        // Asked again meanwhile: the question is up already.
        assert_eq!(ask(true), Reply::Cancel);
        assert!(take());
        reply(false);
        reply(true);
        assert_eq!(replied(), Some(false));
        assert!(!waiting());
        // In a frame, a file dialog open: not now, even with nothing to lose.
        frame_begins();
        UNSAVED.set(false);
        assert_eq!(ask(true), Reply::Cancel);
        frame_ends(false);
        assert!(take());
        assert_eq!(ask(true), Reply::Now);
        assert!(!take());
    }
}
