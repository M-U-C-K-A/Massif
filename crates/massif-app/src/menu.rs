//! The menus — Massif, File, Edit, View and Help — as one list: what each
//! item does, its name, its shortcut, whether it can be used now. On macOS
//! the menu bar at the top of the screen shows the list (`menubar`), the
//! system's own app menu standing for Massif's; elsewhere the window's top
//! bar draws it. The keys are the same either way (`MassifApp::shortcuts`).

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
    /// Another `.massif` (a part of the map sent to someone, or a copy of
    /// it) merged into the map, reviewed first (see `collab`).
    MergeMap,
    ImportPack,
    ExportPack,
    FromHeightMap,
    /// A new world from the relief of a real place.
    FromRealPlace,
    /// A map opened lately.
    OpenRecent(PathBuf),
    Save,
    SaveAs,
    /// Keep the map as it is now under a name (see `versions`).
    SavePoint,
    /// The world's name, seed, sea level, size, memory and blocks.
    WorldInfo,
    ExportHeightMap,
    /// A part of the map as images, to bring back with Import mask.
    ExportRegion,
    Export,
    Quit,
    Undo,
    Redo,
    SelectAll,
    Deselect,
    InvertSelection,
    AutoBiome,
    AutoPaint,
    AutoRules,
    WholeMap(WholeOp),
    /// Edit ▸ Add Reference Image…
    AddReference,
    /// Edit ▸ Whole map ▸ Snowfall…
    Snowfall,
    /// Edit ▸ Smooth terrain…: the top of the land in partial blocks.
    SmoothTerrain,
    Caves,
    /// An image painted onto the map (see `import_mask`).
    ImportMask,
    /// The relief of a real place, into the map.
    RealTerrain,
    /// A script run on the map (see `run_script`).
    RunScript,
    Ores,
    Analyse,
    Theme(ThemeChoice),
    Settings,
    ResetWorkspaces,
    Help,
    Shortcuts,
    /// The guided tour of the editor.
    Tour,
    /// The tour of where WorldPainter's tools are in Massif.
    WorldPainterTour,
    HomeScreen,
    About,
    /// Ask GitHub now whether a newer version is out.
    CheckUpdates,
    /// The window over the whole screen, or back.
    FullScreen,
    /// The whole map in the map view.
    FitMap,
    /// View ▸ Show Annotations: on the map and in 3D, or hidden.
    ShowAnnotations,
    /// A report of a problem to save and send (see `report`).
    ReportProblem,
    /// Start a performance recording, or stop and save it (see `perf`).
    RecordPerformance,
    /// Massif's Discord server, in the browser.
    JoinDiscord,
}

/// Massif's community server on Discord: help, ideas, `#bug-reports`.
pub const DISCORD: &str = "https://discord.gg/2dP7xJb655";

/// Where problems are reported on GitHub.
pub const GITHUB_ISSUES: &str = "https://github.com/M-U-C-K-A/Massif/issues";

/// The title of the first menu, before File, outside macOS (where the
/// system's app menu, named after the app, holds the same).
pub const APP_MENU: &str = "Massif";

/// Where macOS keeps an item: in its menu, or in the app menu (named after
/// the app), where Mac apps have About, Settings and Quit; or nowhere, the
/// system doing it itself (`Elsewhere`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum Role {
    Menu,
    About,
    Settings,
    Quit,
    /// Massif's own menu has it outside macOS; the Mac's app menu, kept
    /// as the system makes it, does not (full screen is the system's
    /// Window menu's there).
    Elsewhere,
}

impl Command {
    /// The action of the keymap that does it, if any (`MassifApp::shortcuts`).
    pub fn action(&self) -> Option<crate::keymap::Action> {
        use crate::keymap::Action;
        Some(match self {
            Command::NewWorld => Action::NewWorld,
            Command::Open => Action::Open,
            Command::Save => Action::Save,
            Command::SaveAs => Action::SaveAs,
            Command::Export => Action::Export,
            Command::Undo => Action::Undo,
            Command::Redo => Action::Redo,
            Command::SelectAll => Action::SelectAll,
            Command::Deselect => Action::Deselect,
            Command::InvertSelection => Action::InvertSelection,
            Command::Settings => Action::Settings,
            Command::Help => Action::Help,
            Command::Quit => Action::Quit,
            Command::FullScreen => Action::FullScreen,
            Command::FitMap => Action::MapFit,
            _ => return None,
        })
    }

    /// The keys that do it, as the keymap has them now, shown beside the
    /// item: ⌘ on a Mac, Ctrl elsewhere.
    pub fn shortcut(&self) -> Option<KeyboardShortcut> {
        crate::keymap::get().shortcut(self.action()?)
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn role(&self) -> Role {
        match self {
            Command::About => Role::About,
            Command::Settings => Role::Settings,
            Command::Quit => Role::Quit,
            Command::CheckUpdates | Command::FullScreen => Role::Elsewhere,
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
    /// The map has a selection: it can be inverted or dropped.
    pub selected: bool,
    /// The annotations are shown.
    pub annotations: bool,
    /// A performance recording is on: the Help menu stops it.
    pub recording: bool,
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
    /// Its keys, as the keymap has them when the menus were made: the menu
    /// bar is made again when they change.
    pub shortcut: Option<KeyboardShortcut>,
}

impl Item {
    fn new(command: Command, icon: Option<Icon>, label: &str, enabled: bool) -> Self {
        Item {
            command: command.clone(),
            label: label.to_owned(),
            icon: icon.map(theme::glyph),
            enabled,
            checked: None,
            tip: None,
            shortcut: command.shortcut(),
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
        item(Command::NewWorld, Icon::FilePlus2, tr!("New World…"), free),
        item(Command::Open, Icon::FolderOpen, tr!("Open…"), free),
        item(Command::ImportWorld, Icon::Import, tr!("Import WorldPainter World…"), free),
        tipped(Command::MergeMap, Icon::GitMerge, tr!("Merge Massif Map…"), map, tr!("Bring in the work done on a copy or a region of this map, reviewed on the map before anything changes")),
        item(Command::FromHeightMap, Icon::ImageUp, tr!("New World from Height Map…"), free),
        item(Command::FromRealPlace, Icon::Earth, tr!("New World from a Real Place…"), free),
        Entry::Submenu {
            label: tr!("Open Recent").to_owned(),
            icon: theme::glyph(Icon::FolderInput),
            enabled: free && !state.recent.is_empty(),
            entries: recent,
        },
        Entry::Separator,
        item(Command::Save, Icon::Save, tr!("Save"), map),
        item(Command::SaveAs, Icon::Save, tr!("Save As…"), map),
        tipped(Command::SavePoint, Icon::BookmarkPlus, tr!("Save Point…"), map, tr!("Keep the map as it is now under a name, to come back to it or compare")),
        tipped(Command::WorldInfo, Icon::Info, tr!("World Information…"), map, tr!("The world's name, seed, sea level and spawn, its blocks, the map's size and the memory it takes")),
        Entry::Separator,
        item(Command::ExportHeightMap, Icon::ImageDown, tr!("Export Height Map…"), map),
        tipped(Command::ExportRegion, Icon::ImageDown, tr!("Export Region…"), map, tr!("A part of the map as a heightmap and masks, to work on in another tool and import back in place")),
        item(Command::Export, Icon::Package, tr!("Export to Minecraft…"), map),
        Entry::Separator,
        tipped(Command::RunScript, Icon::FileCode, tr!("Run Script…"), map, tr!("A script run on the map: one of Massif's, in Lua, or one of WorldPainter's, as it is")),
        Entry::Separator,
        tipped(Command::ImportPack, Icon::Import, tr!("Import Pack…"), free, tr!("Add a .massifpack file someone shared to your packs")),
        tipped(Command::ExportPack, Icon::FileDown, tr!("Export Pack…"), map, tr!("Make a .massifpack file of the map's layers, to share")),
    ];
    let undo = state.undo.map_or_else(|| tr!("Undo").to_owned(), |l| tr!("Undo {l}", l));
    let redo = state.redo.map_or_else(|| tr!("Redo").to_owned(), |l| tr!("Redo {l}", l));
    let edit = vec![
        item(Command::Undo, Icon::Undo2, &undo, map && state.undo.is_some()),
        item(Command::Redo, Icon::Redo2, &redo, map && state.redo.is_some()),
        Entry::Separator,
        tipped(Command::SelectAll, Icon::SquareDashedMousePointer, tr!("Select All"), map, tr!("Select the whole map")),
        tipped(Command::Deselect, Icon::SquareX, tr!("Deselect"), map && state.selected, tr!("The tools act on the whole map again")),
        tipped(Command::InvertSelection, Icon::SquaresExclude, tr!("Invert Selection"), map && state.selected, tr!("Select what is not selected")),
        Entry::Separator,
        tipped(Command::AutoBiome, Icon::Earth, tr!("Auto biome: whole map"), map, tr!("Give every column the biome its land calls for")),
        tipped(Command::AutoPaint, Icon::WandSparkles, tr!("Auto paint: whole map"), map, tr!("Paint every column as its biome calls for")),
        item(Command::AutoRules, Icon::Settings2, tr!("Automatic rules…"), map),
        tipped(Command::SmoothTerrain, Icon::ChartNoAxesColumn, tr!("Smooth terrain…"), map, tr!("The top of the land in partial blocks when it is exported: layers, slabs, stairs, terrain by terrain")),
        Entry::Separator,
        Entry::Submenu {
            label: tr!("Whole map").to_owned(),
            icon: theme::glyph(Icon::Map),
            enabled: map,
            entries: WholeOp::ALL
                .iter()
                .map(|&op| tipped(Command::WholeMap(op), op.icon(), &format!("{}…", crate::i18n::tr_name(op.label())), map, crate::i18n::tr_name(op.about())))
                .chain([Entry::Separator, tipped(Command::Snowfall, Icon::Snowflake, tr!("Snowfall…"), map, tr!("Snow laid by the wind and the slopes: drifts in the lee, bare windward ridges"))])
                .collect(),
        },
        tipped(Command::RealTerrain, Icon::Earth, tr!("Relief of a Real Place…"), map, tr!("The heights of a place on the Earth, into the selection or the whole map")),
        tipped(Command::Caves, Icon::Shovel, tr!("Caves…"), map, tr!("Tunnels, ravines and caverns carved into the rock, over the whole map or the selection")),
        tipped(Command::ImportMask, Icon::ImageUp, tr!("Import Mask…"), map, tr!("An image painted onto the map: annotations, a layer, a terrain, a biome, water or the height")),
        tipped(Command::AddReference, Icon::ImagePlus, tr!("Add Reference Image…"), map, tr!("A picture over or under the map to trace over: shown only, never exported")),
        Entry::Separator,
        tipped(Command::Ores, Icon::Pickaxe, tr!("Ores…"), map, tr!("The ores the export lays in the rock: where, how many, how large")),
        tipped(
            Command::Analyse,
            Icon::ScanSearch,
            tr!("Analyse the map…"),
            map,
            tr!("A survival check of the whole map: every block counted, what breaks survival, what is missing"),
        ),
    ];
    let mut view: Vec<Entry> = [(ThemeChoice::System, tr!("Follow the system")), (ThemeChoice::Light, tr!("Light")), (ThemeChoice::Dark, tr!("Dark"))]
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
        item(Command::FitMap, Icon::Maximize2, tr!("Fit the whole map"), map),
        Entry::Item(Item {
            checked: Some(state.annotations),
            ..Item::new(Command::ShowAnnotations, None, tr!("Show Annotations"), map)
        }),
        Entry::Separator,
        Entry::Item(Item::new(Command::ResetWorkspaces, None, tr!("Reset workspaces"), free)),
    ]);
    let help = vec![
        item(Command::Help, Icon::BookOpen, tr!("Massif Help"), free),
        item(Command::Shortcuts, Icon::Keyboard, tr!("Keyboard shortcuts"), free),
        item(Command::Tour, Icon::Compass, tr!("Take the tour"), free),
        tipped(Command::WorldPainterTour, Icon::Map, tr!("Where are my WorldPainter tools?"), free, tr!("A short tour for those who come from WorldPainter: where each of its tools is in Massif")),
        item(Command::ReportProblem, Icon::Bug, tr!("Report a Problem…"), free),
        if state.recording {
            item(Command::RecordPerformance, Icon::Square, tr!("Stop Recording Performance"), true)
        } else {
            tipped(Command::RecordPerformance, Icon::Activity, tr!("Record Performance…"), free, tr!("Every frame and where its time went, while you use Massif: a file to send us when it slows down"))
        },
        tipped(Command::JoinDiscord, Icon::MessagesSquare, tr!("Join the Discord…"), free, tr!("Massif's community server, in your browser: questions, maps, #bug-reports")),
        Entry::Separator,
        item(Command::HomeScreen, Icon::House, tr!("Home screen"), map),
    ];
    // As the Mac's app menu: About, Settings… and Quit, with what is the
    // application's rather than the map's.
    let app = vec![
        item(Command::About, Icon::Info, tr!("About Massif"), free),
        Entry::Separator,
        item(Command::Settings, Icon::Settings, tr!("Settings…"), free),
        tipped(Command::CheckUpdates, Icon::CircleArrowUp, tr!("Check for Updates…"), free, tr!("Ask GitHub now whether a newer version of Massif is out")),
        Entry::Separator,
        item(Command::FullScreen, Icon::Maximize, tr!("Full Screen"), true),
        item(Command::Quit, Icon::X, tr!("Quit"), true),
    ];
    vec![
        Menu { title: APP_MENU, entries: app },
        Menu { title: tr!("File"), entries: file },
        Menu { title: tr!("Edit"), entries: edit },
        Menu { title: tr!("View"), entries: view },
        Menu { title: tr!("Help"), entries: help },
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
                    if let Some(shortcut) = &item.shortcut {
                        button = button.shortcut_text(ui.ctx().format_shortcut(shortcut));
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
/// menu, the rest in File, Edit, View and Help; Massif's own menu, whose
/// place the app menu takes, not at all.
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
                    Role::Elsewhere => continue,
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
        let entries = tidy(entries);
        if !entries.is_empty() {
            mac.menus.push(Menu { title: menu.title, entries });
        }
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
        // NSHomeFunctionKey.
        None if shortcut.logical_key == Key::Home => "\u{F729}".to_owned(),
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
            selected: true,
            annotations: true,
            recording: false,
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
        assert_eq!(titles, ["Massif", "File", "Edit", "View", "Help"]);
        let labels: Vec<&str> = items(&menus[1].entries).iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "New World…",
                "Open…",
                "Import WorldPainter World…",
                "Merge Massif Map…",
                "New World from Height Map…",
                "New World from a Real Place…",
                "Alps.massif",
                "Coast.massif",
                "Save",
                "Save As…",
                "Save Point…",
                "World Information…",
                "Export Height Map…",
                "Export Region…",
                "Export to Minecraft…",
                "Run Script…",
                "Import Pack…",
                "Export Pack…",
            ]
        );
        assert_eq!(item(&menus, &Command::OpenRecent(recent[1].clone())).tip.as_deref(), Some("/maps/Coast.massif"));
        let undo = item(&menus, &Command::Undo);
        assert!(undo.label == "Undo Raise" && undo.enabled);
        let redo = item(&menus, &Command::Redo);
        assert!(redo.label == "Redo" && !redo.enabled, "nothing to redo");
        assert_eq!(items(&menus[2].entries).iter().filter(|i| matches!(i.command, Command::WholeMap(_))).count(), WholeOp::ALL.len());
        let checked: Vec<&Command> = items(&menus[3].entries).iter().filter(|i| i.checked == Some(true)).map(|i| &i.command).collect();
        assert_eq!(checked, [&Command::Theme(ThemeChoice::Dark), &Command::ShowAnnotations]);
        assert!(all(&menus).iter().all(|i| i.enabled || matches!(i.command, Command::Redo)));
    }

    /// The home screen has no map: what acts on it waits, the ways in and
    /// the rest do not. A dialog holds the window: only Quit is left.
    #[test]
    fn what_can_be_used_when() {
        let recent = [PathBuf::from("/maps/Alps.massif")];
        let home = menus(&State { editor: false, ..state(&recent) });
        for command in [Command::Save, Command::SaveAs, Command::Export, Command::ExportPack, Command::ExportHeightMap, Command::ExportRegion, Command::Undo, Command::Ores, Command::Analyse, Command::HomeScreen, Command::WholeMap(WholeOp::Rivers), Command::Caves, Command::ImportMask, Command::RealTerrain, Command::SavePoint, Command::MergeMap] {
            assert!(!enabled(&home, &command), "{command:?} needs a map");
        }
        for command in [Command::NewWorld, Command::Open, Command::ImportWorld, Command::ImportPack, Command::FromRealPlace, Command::OpenRecent(recent[0].clone()), Command::Settings, Command::Help, Command::About, Command::Quit, Command::Theme(ThemeChoice::Light)] {
            assert!(enabled(&home, &command), "{command:?} works on the home screen");
        }
        let busy = menus(&State { idle: false, ..state(&recent) });
        let usable: Vec<&Command> = all(&busy).into_iter().filter(|i| enabled(&busy, &i.command)).map(|i| &i.command).collect();
        assert_eq!(usable, [&Command::FullScreen, &Command::Quit]);
        // A map no longer in the list cannot be opened from it.
        assert!(!enabled(&menus(&state(&recent)), &Command::OpenRecent(PathBuf::from("/elsewhere.massif"))));
        // No recent map: Open Recent is greyed.
        let none = menus(&state(&[]));
        assert!(matches!(&none[1].entries[6], Entry::Submenu { label, enabled: false, .. } if label == "Open Recent"));
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
                ("Settings…", Some((",".to_owned(), command))),
                ("Full Screen", Some(("\u{F70E}".to_owned(), Modifiers::NONE))),
                ("Quit", Some(("q".to_owned(), command))),
                ("New World…", Some(("n".to_owned(), command))),
                ("Open…", Some(("o".to_owned(), command))),
                ("Save", Some(("s".to_owned(), command))),
                ("Save As…", Some(("s".to_owned(), shifted))),
                ("Export to Minecraft…", Some(("e".to_owned(), command))),
                ("Undo Raise", Some(("z".to_owned(), command))),
                ("Redo", Some(("z".to_owned(), shifted))),
                ("Select All", Some(("a".to_owned(), command))),
                ("Deselect", Some(("d".to_owned(), command))),
                ("Invert Selection", Some(("i".to_owned(), shifted))),
                ("Fit the whole map", Some(("\u{F729}".to_owned(), Modifiers::NONE))),
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
        assert!(matches!(mac.menus[0].entries.last(), Some(Entry::Item(Item { command: Command::ExportPack, .. }))));
        assert!(matches!(mac.menus[3].entries.last(), Some(Entry::Item(Item { command: Command::HomeScreen, .. }))));
        // Massif's own menu is the system's app menu there: nothing of it
        // shows elsewhere, and Check for Updates and Full Screen are not
        // added to the Mac's.
        for command in [Command::CheckUpdates, Command::FullScreen] {
            assert!(all(&mac.menus).iter().all(|i| i.command != command), "{command:?} stays out of the Mac's menus");
        }
    }

    /// On Windows and Linux, Massif's menu comes first, as the Mac's app
    /// menu: About, Settings… (Ctrl+,), Check for Updates…, then Full
    /// Screen (F11) and Quit (Ctrl+Q); View and File no longer hold
    /// Settings and Quit, nor Help About.
    #[test]
    fn windows_and_linux_have_a_massif_menu_first() {
        let menus = menus(&state(&[]));
        assert_eq!(menus[0].title, APP_MENU);
        let app: Vec<(&Command, Option<String>)> = items(&menus[0].entries).iter().map(|i| (&i.command, i.shortcut.as_ref().map(|s| format!("{:?}+{:?}", s.modifiers, s.logical_key)))).collect();
        let command = format!("{:?}", Modifiers::COMMAND);
        assert_eq!(
            app,
            [
                (&Command::About, None),
                (&Command::Settings, Some(format!("{command}+Comma"))),
                (&Command::CheckUpdates, None),
                (&Command::FullScreen, Some(format!("{:?}+F11", Modifiers::NONE))),
                (&Command::Quit, Some(format!("{command}+Q"))),
            ]
        );
        assert!(matches!(menus[0].entries[1], Entry::Separator) && matches!(menus[0].entries[4], Entry::Separator));
        for menu in &menus[1..] {
            for command in [Command::About, Command::Settings, Command::CheckUpdates, Command::FullScreen, Command::Quit] {
                assert!(items(&menu.entries).iter().all(|i| i.command != command), "{} holds {command:?}", menu.title);
            }
            assert!(!matches!(menu.entries.last(), Some(Entry::Separator)), "{} ends with a separator", menu.title);
        }
    }
}
