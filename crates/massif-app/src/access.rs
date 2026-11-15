//! Accessibility: what the system's own settings ask for, and how Massif
//! follows them (Settings ▸ Accessibility).
//!
//! On macOS, System Settings ▸ Accessibility ▸ Display has "Increase
//! contrast" and "Reduce motion": Massif reads them, and follows them unless
//! told otherwise. Elsewhere, "System" is the same as off.

use serde::{Deserialize, Serialize};

/// A setting that can follow the system's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Follow {
    /// As the system says.
    #[default]
    System,
    Off,
    On,
}

impl Follow {
    /// Whether it is on, the system saying `system`.
    pub fn on(self, system: bool) -> bool {
        match self {
            Follow::System => system,
            Follow::Off => false,
            Follow::On => true,
        }
    }
}

/// Whether the system can be followed here.
pub const FOLLOWS_SYSTEM: bool = cfg!(target_os = "macos");

/// What the system asks for: more contrast, and less motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct System {
    pub contrast: bool,
    pub reduce_motion: bool,
}

/// The system's settings, as they are now.
pub fn system() -> System {
    #[cfg(target_os = "macos")]
    {
        mac::read()
    }
    #[cfg(not(target_os = "macos"))]
    {
        System::default()
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::System;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    /// NSWorkspace's accessibility display options. AppKit is loaded by the
    /// window; without it (tests), nothing is asked for.
    pub fn read() -> System {
        let Some(class) = AnyClass::get(c"NSWorkspace") else { return System::default() };
        // SAFETY: class methods and properties of NSWorkspace that take no
        // arguments, returning an object and BOOLs.
        unsafe {
            let workspace: *mut AnyObject = msg_send![class, sharedWorkspace];
            if workspace.is_null() {
                return System::default();
            }
            let contrast: bool = msg_send![workspace, accessibilityDisplayShouldIncreaseContrast];
            let reduce_motion: bool = msg_send![workspace, accessibilityDisplayShouldReduceMotion];
            System { contrast, reduce_motion }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_the_system_or_not() {
        assert!(Follow::System.on(true) && !Follow::System.on(false));
        assert!(Follow::On.on(false) && !Follow::Off.on(true));
        // Reading the system never fails, AppKit loaded or not.
        let _ = system();
    }
}
