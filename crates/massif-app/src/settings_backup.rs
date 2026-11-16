//! A second copy of the settings, written whole or not at all.
//!
//! eframe keeps the settings in `app.ron`, written in place: a full disk
//! or a crash while it writes leaves it empty or cut short, and every
//! setting (saved patterns, the layout, the Minecraft jar) would be lost.
//! The same values are kept in `settings.backup.json` beside it, written
//! through [`massif_core::atomic`] when they change, and read when
//! `app.ron` has lost one.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::PathBuf;

#[derive(Default)]
pub struct SettingsBackup {
    path: Option<PathBuf>,
    values: serde_json::Map<String, serde_json::Value>,
    /// What was written last, to write only when something changed.
    written: String,
}

impl SettingsBackup {
    /// The copy kept beside eframe's settings, read.
    pub fn installed() -> Self {
        Self::at(eframe::storage_dir("massif").map(|d| d.join("settings.backup.json")))
    }

    /// The copy at `path` (`None`: kept nowhere, for tests).
    pub fn at(path: Option<PathBuf>) -> Self {
        let written = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        let values = serde_json::from_str(&written).unwrap_or_default();
        Self { path, values, written }
    }

    /// The value kept under `key`, if it reads as a `T`.
    pub fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.values.get(key).and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Keep `value` under `key`; [`Self::write`] writes them.
    pub fn set<T: Serialize>(&mut self, key: &str, value: &T) {
        if let Ok(v) = serde_json::to_value(value) {
            self.values.insert(key.to_owned(), v);
        }
    }

    /// Write the values if they changed since last written.
    pub fn write(&mut self) {
        let Some(path) = &self.path else { return };
        let Ok(text) = serde_json::to_string_pretty(&self.values) else { return };
        if text == self.written {
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match massif_core::atomic::write(path, text.as_bytes(), true) {
            Ok(()) => self.written = text,
            Err(e) => log::warn!("the settings' backup could not be written: {}", massif_core::atomic::describe(&e, path)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_come_back_from_the_copy() {
        let dir = std::env::temp_dir().join(format!("massif-settings-backup-{}", std::process::id()));
        let path = dir.join("settings.backup.json");
        let mut b = SettingsBackup::at(Some(path.clone()));
        assert_eq!(b.get::<Vec<u32>>("list"), None);
        b.set("list", &vec![1u32, 2, 3]);
        b.set("name", &"Northern woods");
        b.write();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        b.write();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), modified, "nothing written when nothing changed");
        let back = SettingsBackup::at(Some(path.clone()));
        assert_eq!(back.get::<Vec<u32>>("list"), Some(vec![1, 2, 3]));
        assert_eq!(back.get::<String>("name").as_deref(), Some("Northern woods"));
        assert_eq!(back.get::<String>("list"), None);
        // A copy cut short reads as none, without a panic.
        std::fs::write(&path, b"{\"list\": [1, 2").unwrap();
        assert_eq!(SettingsBackup::at(Some(path)).get::<Vec<u32>>("list"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
