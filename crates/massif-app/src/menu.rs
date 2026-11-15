//! The menus — File, Edit, View and Help — as one list: what each item
//! does, its name, its shortcut, whether it can be used now. On macOS the
//! menu bar at the top of the screen shows the list (`menubar`); elsewhere
//! the window's top bar draws it. The keys are the same either way
//! (`MassifApp::shortcuts`).

use crate::app::ThemeChoice;
use crate::reshape::WholeOp;
use crate::theme::{self, Icon};
use egui::{Key, KeyboardShortcut, Modifiers, Ui};
use std::path::PathBuf;

/// What an item of the menus does.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    NewWorld,
    Open,
    ImportWorld,
    FromHeightMap,
    /// A map opened lately.
    OpenRecent(PathBuf),
    Save,
    SaveAs,
    ExportHeightMap,
    Export,
    Quit,
    Undo,
    Redo,
    AutoBiome,
    AutoPaint,
    AutoRules,
    WholeMap(WholeOp),
    Ores,
    Analyse,
    Theme(ThemeChoice),
    Settings,
    ResetWorkspaces,
    Help,
    Shortcuts,
    HomeScreen,
    About,
}

/// Where macOS keeps an item: in its menu, or in the app menu (named after
/// the app), where Mac apps have About, Settings and Quit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum Role {
    Menu,
    About,
    Settings,
    Quit,
}

impl Command {
    /// The keys that do it, handled by the app (`MassifApp::shortcuts`) and
    /// shown beside the item: ⌘ on a Mac, Ctrl elsewhere.
    pub fn shortcut(&self) -> Option<KeyboardShortcut> {
        let command = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let shifted = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        Some(match self {
            Command::NewWorld => command(Key::N),
            Command::Open => command(Key::O),
            Command::Save => command(Key::S),
            Command::SaveAs => shifted(Key::S),
            Command::Export => command(Key::E),
            Command::Undo => command(Key::Z),
            Command::Redo => shifted(Key::Z),
            Command::Settings => command(Key::Comma),
            Command::Help => KeyboardShortcut::new(Modifiers::NONE, Key::F1),
            _ => return None,
        })
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn role(&self) -> Role {
        match self {
            Command::About => Role::About,
            Command::Settings => Role::Settings,
            Command::Quit => Role::Quit,
            _ => Role::Menu,
        }
    }
}

/// What the menus depend on.
pub struct State<'a> {
    /// The editor is shown, not the home screen: what acts on the map can
    /// be done.
    pub editor: bool,
    /// Nothing holds the window — no dialog, no map being opened or made —
    /// so the menus can be used. Quit always can.
    pub idle: bool,
    /// What Undo and Redo would take back and do again.
    pub undo: Option<&'a str>,
    pub redo: Option<&'a str>,
    /// The maps opened lately.
    pub recent: &'a [PathBuf],
    pub theme: ThemeChoice,
}

/// An item: a command under a name.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub command: Command,
    pub label: String,
    /// The glyph of its icon, in the window's menus.
    pub icon: Option<char>,
    pub enabled: bool,
    /// One choice among others (the theme): whether it is the one chosen.
    pub checked: Option<bool>,
    /// What it does, when its name does not say it all.
    pub tip: Option<String>,
}

impl Item {
    fn new(command: Command, icon: Option<Icon>, label: &str, enabled: bool) -> Self {
        Item {
            command,
            label: label.to_owned(),
            icon: icon.map(theme::glyph),
            enabled,
            checked: None,
            tip: None,
        }
    }

    fn tip(self, tip: impl Into<String>) -> Self {
        Item { tip: Some(tip.into()), ..self }
    }

    /// Its name after its icon, as the window's menus show it.
    fn text(&self) -> String {
        match self.icon {
            Some(glyph) => format!("{glyph}  {}", self.label),
            None => self.label.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Item(Item),
    /// Items under a name of their own.
    Submenu { label: String, icon: char, enabled: bool, entries: Vec<Entry> },
    Separator,
}

/// A menu of the bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub title: &'static str,
    pub entries: Vec<Entry>,
}

/// The menus, as they are now.
pub fn menus(state: &State) -> Vec<Menu> {
    // What acts on the map, and what needs no map (the home screen offers
    // it too).
    let map = state.editor && state.idle;
    let free = state.idle;
    let item = |command, icon, label: &str, enabled| Entry::Item(Item::new(command, Some(icon), label, enabled));
    let tipped = |command, icon, label: &str, enabled, tip: &str| Entry::Item(Item::new(command, Some(icon), label, enabled).tip(tip));
    let recent = state
        .recent
        .iter()
        .map(|path| Entry::Item(Item::new(Command::OpenRecent(path.clone()), None, &crate::jobs::display_name(path), free).tip(path.display().to_string())))
        .collect();
    let file = vec![
        item(Command::NewWorld, Icon::FilePlus2, "New World…", free),
        item(Command::Open, Icon::FolderOpen, "Open…", free),
        item(Command::ImportWorld, Icon::Import, "Import WorldPainter World…", free),
        item(Command::FromHeightMap, Icon::ImageUp, "New World from Height Map…", free),
        Entry::Submenu {
            label: "Open Recent".to_owned(),
            icon: theme::glyph(Icon::FolderInput),
            enabled: free && !state.recent.is_empty(),
            entries: recent,
        },
        Entry::Separator,
        item(Command::Save, Icon::Save, "Save", map),
        item(Command::SaveAs, Icon::Save, "Save As…", map),
        Entry::Separator,
        item(Command::ExportHeightMap, Icon::ImageDown, "Export Height Map…", map),
        item(Command::Export, Icon::Package, "Export to Minecraft…", map),
        Entry::Separator,
        item(Command::Quit, Icon::X, "Quit", true),
    ];
    let undo = state.undo.map_or_else(|| "Undo".to_owned(), |l| format!("Undo {l}"));
    let redo = state.redo.map_or_else(|| "Redo".to_owned(), |l| format!("Redo {l}"));
    let edit = vec![
        item(Command::Undo, Icon::Undo2, &undo, map && state.undo.is_some()),
        item(Command::Redo, Icon::Redo2, &redo, map && state.redo.is_some()),
        Entry::Separator,
        tipped(Command::AutoBiome, Icon::Earth, "Auto biome: whole map", map, "Give every column the biome its land calls for"),
        tipped(Command::AutoPaint, Icon::WandSparkles, "Auto paint: whole map", map, "Paint every column as its biome calls for"),
        item(Command::AutoRules, Icon::Settings2, "Automatic rules…", map),
        Entry::Separator,
        Entry::Submenu {
            label: "Whole map".to_owned(),
            icon: theme::glyph(Icon::Map),
            enabled: map,
            entries: WholeOp::ALL
                .iter()
                .map(|&op| tipped(Command::WholeMap(op), op.icon(), &format!("{}…", op.label()), map, op.about()))
                .collect(),
        },
        Entry::Separator,
        tipped(Command::Ores, Icon::Pickaxe, "Ores…", map, "The ores the export lays in the rock: where, how many, how large"),
        tipped(
            Command::Analyse,
            Icon::ScanSearch,
            "Analyse the map…",
            map,
            "A survival check of the whole map: every block counted, what breaks survival, what is missing",
        ),
    ];
    let mut view: Vec<Entry> = [(ThemeChoice::System, "Follow the system"), (ThemeChoice::Light, "Light"), (ThemeChoice::Dark, "Dark")]
        .into_iter()
        .map(|(choice, label)| {
            Entry::Item(Item {
                checked: Some(state.theme == choice),
                ..Item::new(Command::Theme(choice), None, label, free)
            })
        })
        .collect();
    view.extend([
        Entry::Separator,
        item(Command::Settings, Icon::Settings, "Settings…", free),
        Entry::Item(Item::new(Command::ResetWorkspaces, None, "Reset workspaces", free)),
    ]);
    let help = vec![
        item(Command::Help, Icon::BookOpen, "Massif Help", free),
        item(Command::Shortcuts, Icon::Keyboard, "Keyboard shortcuts", free),
        Entry::Separator,
        item(Command::HomeScreen, Icon::House, "Home screen", map),
        item(Command::About, Icon::Info, "About Massif", free),
    ];
    vec![
        Menu { title: "File", entries: file },
        Menu { title: "Edit", entries: edit },
        Menu { title: "View", entries: view },
        Menu { title: "Help", entries: help },
    ]
}

/// Whether `command` can be picked in `menus` now: its item is there,
/// enabled, and so is the submenu it is in.
pub fn enabled(menus: &[Menu], command: &Command) -> bool {
    fn find(entries: &[Entry], command: &Command) -> bool {
        entries.iter().any(|entry| match entry {
            Entry::Item(item) => item.enabled && item.command == *command,
            Entry::Submenu { enabled, entries, .. } => *enabled && find(entries, command),
            Entry::Separator => false,
        })
    }
    menus.iter().any(|menu| find(&menu.entries, command))
}

/// The entries of a menu in egui's menus; the command of the one clicked.
pub fn ui(ui: &mut Ui, entries: &[Entry]) -> Option<Command> {
    let mut picked = None;
    for entry in entries {
        match entry {
            Entry::Separator => {
                ui.separator();
            }
            Entry::Item(item) => {
                let response = if let Some(checked) = item.checked {
                    ui.add_enabled(item.enabled, egui::RadioButton::new(checked, item.text()))
                } else {
                    let mut button = egui::Button::new(item.text());
                    if let Some(shortcut) = item.command.shortcut() {
                        button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
                    }
                    ui.add_enabled(item.enabled, button)
                };
                let response = match &item.tip {
                    Some(tip) => response.on_hover_text(tip),
                    None => response,
                };
                if response.clicked() {
                    picked = Some(item.command.clone());
                }
            }
            Entry::Submenu { label, icon, enabled, entries } => {
                ui.add_enabled_ui(*enabled, |ui| {
                    ui.menu_button(format!("{icon}  {label}"), |ui| {
                        if let Some(command) = self::ui(ui, entries) {
                            picked = Some(command);
                        }
                    });
                });
            }
        }
    }
    picked
}

/// The menus as macOS shows them: About, Settings… and Quit in the app
/// menu, the rest in File, Edit, View and Help.
#[derive(Debug, Default, PartialEq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct MacMenus {
    pub about: Option<Item>,
    pub settings: Option<Item>,
    pub quit: Option<Item>,
    pub menus: Vec<Menu>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn for_macos(menus: &[Menu]) -> MacMenus {
    let mut mac = MacMenus::default();
    for menu in menus {
        let mut entries = Vec::with_capacity(menu.entries.len());
        for entry in &menu.entries {
            if let Entry::Item(item) = entry {
                let slot = match item.command.role() {
                    Role::Menu => None,
                    Role::About => Some(&mut mac.about),
                    Role::Settings => Some(&mut mac.settings),
                    Role::Quit => Some(&mut mac.quit),
                };
                if let Some(slot) = slot {
                    *slot = Some(item.clone());
                    continue;
                }
            }
            entries.push(entry.clone());
        }
        mac.menus.push(Menu { title: menu.title, entries: tidy(entries) });
    }
    mac
}

/// No separator first, last or twice in a row (where items were taken out).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn tidy(entries: Vec<Entry>) -> Vec<Entry> {
    let mut tidy: Vec<Entry> = Vec::with_capacity(entries.len());
    for entry in entries {
        if matches!(entry, Entry::Separator) && tidy.last().is_none_or(|last| matches!(last, Entry::Separator)) {
            continue;
        }
        tidy.push(entry);
    }
    if matches!(tidy.last(), Some(Entry::Separator)) {
        tidy.pop();
    }
    tidy
}

/// A shortcut as a macOS menu item holds it: the character of its key —
/// lower case; function keys have their own, in Unicode's private use area
/// — and its modifiers.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn key_equivalent(shortcut: &KeyboardShortcut) -> Option<(String, Modifiers)> {
    const FUNCTION: [Key; 12] = [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12];
    let key = match FUNCTION.iter().position(|k| *k == shortcut.logical_key) {
        // NSF1FunctionKey and on.
        Some(n) => char::from_u32(0xF704 + n as u32)?.to_string(),
        // egui draws the minus sign for −: the key types a hyphen.
        None if shortcut.logical_key == Key::Minus => "-".to_owned(),
        None => {
            let mut chars = shortcut.logical_key.symbol_or_name().chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            c.to_lowercase().to_string()
        }
    };
    Some((key, shortcut.modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(recent: &[PathBuf]) -> State<'_> {
        State {
            editor: true,
            idle: true,
            undo: Some("Raise"),
            redo: None,
            recent,
            theme: ThemeChoice::Dark,
        }
    }

    fn items(entries: &[Entry]) -> Vec<&Item> {
        entries
            .iter()
            .flat_map(|entry| match entry {
                Entry::Item(item) => vec![item],
                Entry::Submenu { entries, .. } => items(entries),
                Entry::Separator => Vec::new(),
            })
            .collect()
    }

    fn all(menus: &[Menu]) -> Vec<&Item> {
        menus.iter().flat_map(|m| items(&m.entries)).collect()
    }

    fn item<'a>(menus: &'a [Menu], command: &Command) -> &'a Item {
        all(menus).into_iter().find(|i| i.command == *command).unwrap_or_else(|| panic!("{command:?} is in the menus"))
    }

    /// The menus hold what the top bar always held, in its order, with the
    /// history's labels, the recent maps and the theme chosen.
    #[test]
    fn the_menus_and_their_items() {
        let recent = [PathBuf::from("/maps/Alps.massif"), PathBuf::from("/maps/Coast.massif")];
        let menus = menus(&state(&recent));
        let titles: Vec<&str> = menus.iter().map(|m| m.title).collect();
        assert_eq!(titles, ["File", "Edit", "View", "Help"]);
        let labels: Vec<&str> = items(&menus[0].entries).iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "New World…",
                "Open…",
                "Import WorldPainter World…",
                "New World from Height Map…",
                "Alps.massif",
                "Coast.massif",
                "Save",
                "Save As…",
                "Export Height Map…",
                "Export to Minecraft…",
                "Quit"
            ]
        );
        assert_eq!(item(&menus, &Command::OpenRecent(recent[1].clone())).tip.as_deref(), Some("/maps/Coast.massif"));
        let undo = item(&menus, &Command::Undo);
        assert!(undo.label == "Undo Raise" && undo.enabled);
        let redo = item(&menus, &Command::Redo);
        assert!(redo.label == "Redo" && !redo.enabled, "nothing to redo");
        assert_eq!(items(&menus[1].entries).iter().filter(|i| matches!(i.command, Command::WholeMap(_))).count(), WholeOp::ALL.len());
        let checked: Vec<&Command> = items(&menus[2].entries).iter().filter(|i| i.checked == Some(true)).map(|i| &i.command).collect();
        assert_eq!(checked, [&Command::Theme(ThemeChoice::Dark)]);
        assert!(all(&menus).iter().all(|i| i.enabled || matches!(i.command, Command::Redo)));
    }

    /// The home screen has no map: what acts on it waits, the ways in and
    /// the rest do not. A dialog holds the window: only Quit is left.
    #[test]
    fn what_can_be_used_when() {
        let recent = [PathBuf::from("/maps/Alps.massif")];
        let home = menus(&State { editor: false, ..state(&recent) });
        for command in [Command::Save, Command::SaveAs, Command::Export, Command::ExportHeightMap, Command::Undo, Command::Ores, Command::Analyse, Command::HomeScreen, Command::WholeMap(WholeOp::Rivers)] {
            assert!(!enabled(&home, &command), "{command:?} needs a map");
        }
        for command in [Command::NewWorld, Command::Open, Command::ImportWorld, Command::OpenRecent(recent[0].clone()), Command::Settings, Command::Help, Command::About, Command::Quit, Command::Theme(ThemeChoice::Light)] {
            assert!(enabled(&home, &command), "{command:?} works on the home screen");
        }
        let busy = menus(&State { idle: false, ..state(&recent) });
        let usable: Vec<&Command> = all(&busy).into_iter().filter(|i| enabled(&busy, &i.command)).map(|i| &i.command).collect();
        assert_eq!(usable, [&Command::Quit]);
        // A map no longer in the list cannot be opened from it.
        assert!(!enabled(&menus(&state(&recent)), &Command::OpenRecent(PathBuf::from("/elsewhere.massif"))));
        // No recent map: Open Recent is greyed.
        let none = menus(&state(&[]));
        assert!(matches!(&none[0].entries[4], Entry::Submenu { label, enabled: false, .. } if label == "Open Recent"));
    }

    /// The shortcuts shown are the ones `MassifApp::shortcuts` handles, as
    /// macOS writes them on its menu items.
    #[test]
    fn shortcuts_and_their_keys() {
        let menus = menus(&state(&[]));
        let shown: Vec<(&str, Option<(String, Modifiers)>)> = all(&menus)
            .into_iter()
            .filter_map(|i| i.command.shortcut().map(|s| (i.label.as_str(), key_equivalent(&s))))
            .collect();
        let command = Modifiers::COMMAND;
        let shifted = Modifiers::COMMAND | Modifiers::SHIFT;
        assert_eq!(
            shown,
            [
                ("New World…", Some(("n".to_owned(), command))),
                ("Open…", Some(("o".to_owned(), command))),
                ("Save", Some(("s".to_owned(), command))),
                ("Save As…", Some(("s".to_owned(), shifted))),
                ("Export to Minecraft…", Some(("e".to_owned(), command))),
                ("Undo Raise", Some(("z".to_owned(), command))),
                ("Redo", Some(("z".to_owned(), shifted))),
                ("Settings…", Some((",".to_owned(), command))),
                ("Massif Help", Some(("\u{F704}".to_owned(), Modifiers::NONE))),
            ]
        );
        assert_eq!(key_equivalent(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Minus)).map(|k| k.0).as_deref(), Some("-"));
        assert_eq!(key_equivalent(&KeyboardShortcut::new(Modifiers::NONE, Key::F12)).map(|k| k.0).as_deref(), Some("\u{F70F}"));
        assert_eq!(key_equivalent(&KeyboardShortcut::new(Modifiers::NONE, Key::Escape)), None, "no character of its own");
    }

    /// On macOS, About, Settings… and Quit go to the app menu, and no
    /// separator is left hanging where they were.
    #[test]
    fn macos_keeps_about_settings_and_quit_in_the_app_menu() {
        let menus = menus(&state(&[]));
        let mac = for_macos(&menus);
        assert_eq!(mac.about.as_ref().map(|i| i.label.as_str()), Some("About Massif"));
        assert_eq!(mac.settings.as_ref().map(|i| &i.command), Some(&Command::Settings));
        assert!(mac.quit.as_ref().is_some_and(|i| i.command == Command::Quit && i.enabled));
        let titles: Vec<&str> = mac.menus.iter().map(|m| m.title).collect();
        assert_eq!(titles, ["File", "Edit", "View", "Help"]);
        for menu in &mac.menus {
            assert!(items(&menu.entries).iter().all(|i| i.command.role() == Role::Menu), "{} keeps no app menu item", menu.title);
            assert!(!matches!(menu.entries.first(), Some(Entry::Separator)) && !matches!(menu.entries.last(), Some(Entry::Separator)), "{}", menu.title);
            assert!(!menu.entries.windows(2).any(|w| matches!(w, [Entry::Separator, Entry::Separator])), "{}", menu.title);
        }
        // Everything else is where the window's menus have it.
        let kept = |m: &[Menu]| -> Vec<Command> { all(m).into_iter().filter(|i| i.command.role() == Role::Menu).map(|i| i.command.clone()).collect() };
        assert_eq!(kept(&mac.menus), kept(&menus));
        assert!(matches!(mac.menus[0].entries.last(), Some(Entry::Item(Item { command: Command::Export, .. }))));
        assert!(matches!(mac.menus[3].entries.last(), Some(Entry::Item(Item { command: Command::HomeScreen, .. }))));
    }
}
