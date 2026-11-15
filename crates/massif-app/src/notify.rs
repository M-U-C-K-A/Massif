//! Notices from the system — the Notification Centre on a Mac, a toast on
//! Windows, the desktop's notifications on Linux — when a long task ends,
//! so that it is known while Massif is behind other windows.

use serde::{Deserialize, Serialize};

/// When the system tells that a long task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotifyWhen {
    Never,
    /// While Massif is not the window in front.
    Background,
    Always,
}

/// Ask the system to show `title` and `body`; on a thread of its own, as
/// the tools it goes through take a moment.
pub fn send(title: &str, body: &str) {
    if cfg!(test) {
        return;
    }
    let (title, body) = (title.to_owned(), body.to_owned());
    let _ = std::thread::Builder::new().name("massif notice".into()).spawn(move || {
        if let Err(e) = show(&title, &body) {
            log::warn!("no notification: {e}");
        }
    });
}

#[cfg(target_os = "macos")]
fn show(title: &str, body: &str) -> std::io::Result<()> {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let script = format!("display notification {} with title {}", quote(body), quote(title));
    std::process::Command::new("osascript").args(["-e", &script]).output().map(|_| ())
}

#[cfg(target_os = "linux")]
fn show(title: &str, body: &str) -> std::io::Result<()> {
    std::process::Command::new("notify-send").args(["--app-name=Massif", title, body]).output().map(|_| ())
}

#[cfg(target_os = "windows")]
fn show(title: &str, body: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    // A toast through PowerShell's own application id, with no window.
    let script = format!(
        "$ErrorActionPreference='SilentlyContinue';\
         [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null;\
         $t=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02);\
         $x=$t.GetElementsByTagName('text');\
         $x.Item(0).AppendChild($t.CreateTextNode({})) | Out-Null;\
         $x.Item(1).AppendChild($t.CreateTextNode({})) | Out-Null;\
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($t))",
        quote(title),
        quote(body)
    );
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).creation_flags(CREATE_NO_WINDOW).output().map(|_| ())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn show(_title: &str, _body: &str) -> std::io::Result<()> {
    Ok(())
}
