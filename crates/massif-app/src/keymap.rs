//! The keyboard shortcuts, in one table: every action the keys do, its id
//! (what a keymap file and the settings name it by), its area, and its keys
//! in each preset — "Massif", the default, and "WorldPainter", that app's
//! own. The user's changes are kept apart from the preset (`Saved`), by id:
//! an id this version does not know is left aside, and an action new in
//! this version has its preset's keys.
//!
//! The keys of a frame are read once (`take`), each key press matched to
//! the actions it is bound to — the binding that names exactly the keys
//! held first, then one that leaves Shift out (Shift + − makes the brush a
//! block smaller), then the same by the key's place on the keyboard (on an
//! AZERTY keyboard the top row types & é " …, and 1 … 0 still set the
//! intensity). While a text field is typed in, only the actions with ⌘ (Ctrl)
//! that typing has no use for, Save or Open, are done. Keys held down —
//! flying, walking, Space to move the map, Shift and Alt while selecting —
//! are asked for where they are used (`held`, `mods_hold`).
//!
//! The mouse keeps its own: the wheel sizes the brush, ⌘ (Ctrl) or Alt and
//! the wheel zoom, ⌘ and the wheel turns the brush.

use crate::tools::Tool;
use egui::{Key, KeyboardShortcut, Modifiers};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Whether shortcuts are shown and read as a Mac's: ⌘ ⌥ ⇧ ⌃.
pub const MAC: bool = cfg!(target_os = "macos");

/// The modifiers of a shortcut. `command` is ⌘ on a Mac and Ctrl elsewhere;
/// `ctrl` is the Control key of a Mac (elsewhere it is `command`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct Mods {
    pub command: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { command: false, ctrl: false, alt: false, shift: false };
    pub const COMMAND: Mods = Mods { command: true, ..Mods::NONE };
    pub const CTRL: Mods = Mods { ctrl: true, ..Mods::NONE };
    pub const ALT: Mods = Mods { alt: true, ..Mods::NONE };
    pub const SHIFT: Mods = Mods { shift: true, ..Mods::NONE };
    pub const COMMAND_SHIFT: Mods = Mods { command: true, shift: true, ..Mods::NONE };

    /// The modifiers egui reports, as this system has them.
    pub fn of(m: Modifiers) -> Mods {
        if MAC {
            Mods { command: m.mac_cmd || (m.command && !m.ctrl), ctrl: m.ctrl, alt: m.alt, shift: m.shift }
        } else {
            Mods { command: m.command || m.ctrl, ctrl: false, alt: m.alt, shift: m.shift }
        }
        .normal()
    }

    /// Off a Mac, Control is the command key.
    fn normal(self) -> Mods {
        if MAC || !self.ctrl { self } else { Mods { command: true, ctrl: false, ..self } }
    }

    pub fn is_none(self) -> bool {
        self == Mods::NONE
    }

    /// Every modifier of `self` is in `other`.
    fn within(self, other: Mods) -> bool {
        (!self.command || other.command) && (!self.ctrl || other.ctrl) && (!self.alt || other.alt) && (!self.shift || other.shift)
    }

    fn egui(self) -> Modifiers {
        let mut m = Modifiers::NONE;
        if self.command {
            m |= Modifiers::COMMAND;
        }
        if self.ctrl {
            m |= Modifiers::CTRL;
        }
        if self.alt {
            m |= Modifiers::ALT;
        }
        if self.shift {
            m |= Modifiers::SHIFT;
        }
        m
    }
}

/// A shortcut: modifiers and a key, or modifiers alone (for what is done
/// while they are held: Shift adds to a selection).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub mods: Mods,
    pub key: Option<Key>,
}

impl Chord {
    pub const fn key(key: Key) -> Chord {
        Chord { mods: Mods::NONE, key: Some(key) }
    }

    pub const fn with(mods: Mods, key: Key) -> Chord {
        Chord { mods, key: Some(key) }
    }

    pub const fn mods(mods: Mods) -> Chord {
        Chord { mods, key: None }
    }

    /// As a keymap file writes it, the same on every system: `Cmd+Shift+Z`,
    /// `Alt+H`, `Ctrl+Space`, `Shift`.
    pub fn id(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let m = self.mods;
        for (on, name) in [(m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.command, "Cmd")] {
            if on {
                parts.push(name);
            }
        }
        if let Some(key) = self.key {
            parts.push(key.name());
        }
        parts.join("+")
    }

    /// A shortcut as `id` writes it; `None` when a part is not a key egui
    /// knows. `Plus` is the key; a `+` between parts only separates them.
    pub fn parse(text: &str) -> Option<Chord> {
        let mut mods = Mods::NONE;
        let mut key = None;
        for part in text.split('+').map(str::trim) {
            match part.to_ascii_lowercase().as_str() {
                "" => return None,
                "cmd" | "command" | "meta" | "super" => mods.command = true,
                "ctrl" | "control" => mods.ctrl = true,
                "alt" | "option" | "opt" => mods.alt = true,
                "shift" => mods.shift = true,
                _ => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(Key::from_name(part)?);
                }
            }
        }
        let chord = Chord { mods: mods.normal(), key };
        (key.is_some() || !mods.is_none()).then_some(chord)
    }

    /// As the interface shows it: ⇧⌘Z on a Mac, Ctrl+Shift+Z elsewhere.
    pub fn label(&self) -> String {
        self.label_for(MAC)
    }

    pub fn label_for(&self, mac: bool) -> String {
        let m = self.mods;
        if mac {
            let mut s = String::new();
            for (on, sign) in [(m.ctrl, '⌃'), (m.alt, '⌥'), (m.shift, '⇧'), (m.command, '⌘')] {
                if on {
                    s.push(sign);
                }
            }
            if let Some(key) = self.key {
                s.push_str(&key_label(key, true));
            } else if s.chars().count() == 1 {
                // A modifier alone: by its name, a sign is hard to read alone.
                s = [(m.ctrl, "Control"), (m.alt, "Option"), (m.shift, "Shift"), (m.command, "Command")].iter().find(|(on, _)| *on).map(|(_, n)| (*n).to_owned()).unwrap_or(s);
            }
            s
        } else {
            let mut parts: Vec<String> = Vec::new();
            for (on, name) in [(m.command || m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift")] {
                if on {
                    parts.push(name.to_owned());
                }
            }
            if let Some(key) = self.key {
                parts.push(key_label(key, false));
            }
            parts.join("+")
        }
    }

    /// As egui's menus and macOS's menu bar take it: a key with modifiers.
    pub fn shortcut(&self) -> Option<KeyboardShortcut> {
        Some(KeyboardShortcut::new(self.mods.egui(), self.key?))
    }
}

/// A key's name as shown: a sign where it is the usual one.
fn key_label(key: Key, mac: bool) -> String {
    match key {
        Key::ArrowLeft => "←".into(),
        Key::ArrowRight => "→".into(),
        Key::ArrowUp => "↑".into(),
        Key::ArrowDown => "↓".into(),
        Key::Escape => "Esc".into(),
        Key::Enter if mac => "↩".into(),
        Key::Backspace if mac => "⌫".into(),
        Key::Delete if mac => "⌦".into(),
        Key::Tab if mac => "⇥".into(),
        Key::PageUp => tr!("Page Up").into(),
        Key::PageDown => tr!("Page Down").into(),
        Key::Space => tr!("Space").into(),
        Key::Minus => "−".into(),
        _ => key.symbol_or_name().to_owned(),
    }
}

/// The groups of actions, one column each in Settings ▸ Keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    General,
    Tools,
    Brushes,
    Map,
    View3d,
    Player,
}

impl Area {
    pub const ALL: [Area; 6] = [Area::General, Area::Tools, Area::Brushes, Area::Map, Area::View3d, Area::Player];

    pub fn name(self) -> &'static str {
        match self {
            Area::General => tr!("General"),
            Area::Tools => tr!("Tools"),
            Area::Brushes => tr!("Brushes"),
            Area::Map => tr!("Map"),
            Area::View3d => tr!("3D view"),
            Area::Player => tr!("Player"),
        }
    }
}

/// Where an action's keys are listened to: two actions with the same keys
/// clash only where both are listened to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Anywhere in the window.
    Global,
    /// Over the map.
    Map,
    /// Flying over the 3D view.
    Fly,
    /// In 3D Sculpt's Player.
    Player,
}

impl Scope {
    fn meets(self, other: Scope) -> bool {
        self == other || self == Scope::Global || other == Scope::Global
    }
}

/// How an action takes its keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Done once when its keys are pressed.
    Press,
    /// Goes on while its keys are held: a key, or modifiers alone.
    Hold,
    /// A modifier held while the mouse acts: modifiers alone.
    Modifier,
}

/// What the keys do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    NewWorld,
    Open,
    Save,
    SaveAs,
    Export,
    Undo,
    Redo,
    Settings,
    Help,
    UiZoomIn,
    UiZoomOut,
    UiZoomReset,
    MaximizeArea,
    Delete,
    Cancel,
    Tool(Tool),
    /// The brush's intensity, in tenths: 1 is 10 %, 10 is 100 %.
    Intensity(u8),
    BrushSmaller,
    BrushLarger,
    TurnLeft,
    TurnRight,
    TurnReset,
    Turn30,
    Turn45,
    Turn90,
    LightRight,
    LightLeft,
    Pan,
    SelectAll,
    Deselect,
    InvertSelection,
    SelectAdd,
    SelectSubtract,
    MapZoomIn,
    MapZoomOut,
    MapZoomReset,
    FlyForward,
    FlyBack,
    FlyLeft,
    FlyRight,
    FlyDown,
    FlyUp,
    FlyFaster,
    WalkForward,
    WalkBack,
    WalkLeft,
    WalkRight,
    Jump,
    Sneak,
    Sprint,
    LookLeft,
    LookRight,
    LookUp,
    LookDown,
    ReleaseMouse,
}

/// The tools a shortcut can pick, in the order of the toolbox, then 3D
/// Sculpt's.
pub fn tools() -> Vec<Tool> {
    let mut list: Vec<Tool> = Tool::GROUPS.iter().flat_map(|(_, _, tools)| tools.iter().copied()).collect();
    list.extend([Tool::Build, Tool::Dig, Tool::Smooth3d]);
    list
}

impl Action {
    /// Every action, area by area.
    pub fn all() -> Vec<Action> {
        use Action::*;
        let mut list = vec![NewWorld, Open, Save, SaveAs, Export, Undo, Redo, Delete, Cancel, Settings, Help, UiZoomIn, UiZoomOut, UiZoomReset, MaximizeArea];
        list.extend(tools().into_iter().map(Tool));
        list.extend((1..=10).map(Intensity));
        list.extend([BrushSmaller, BrushLarger, TurnLeft, TurnRight, TurnReset, Turn30, Turn45, Turn90]);
        list.extend([Pan, MapZoomIn, MapZoomOut, MapZoomReset, LightRight, LightLeft, SelectAll, Deselect, InvertSelection, SelectAdd, SelectSubtract]);
        list.extend([FlyForward, FlyBack, FlyLeft, FlyRight, FlyUp, FlyDown, FlyFaster]);
        list.extend([WalkForward, WalkBack, WalkLeft, WalkRight, Jump, Sneak, Sprint, LookLeft, LookRight, LookUp, LookDown, ReleaseMouse]);
        list
    }

    /// Its name in keymap files and the settings, never changed once out.
    pub fn id(self) -> String {
        use Action::*;
        let s = match self {
            Tool(tool) => return format!("tool.{}", format!("{tool:?}").to_ascii_lowercase()),
            Intensity(n) => return format!("brush.intensity_{}", n as u32 * 10),
            NewWorld => "file.new",
            Open => "file.open",
            Save => "file.save",
            SaveAs => "file.save_as",
            Export => "file.export",
            Undo => "edit.undo",
            Redo => "edit.redo",
            Settings => "app.settings",
            Help => "app.help",
            UiZoomIn => "app.zoom_in",
            UiZoomOut => "app.zoom_out",
            UiZoomReset => "app.zoom_reset",
            MaximizeArea => "app.maximize_area",
            Delete => "edit.delete",
            Cancel => "edit.cancel",
            BrushSmaller => "brush.smaller",
            BrushLarger => "brush.larger",
            TurnLeft => "brush.turn_left",
            TurnRight => "brush.turn_right",
            TurnReset => "brush.turn_reset",
            Turn30 => "brush.turn_30",
            Turn45 => "brush.turn_45",
            Turn90 => "brush.turn_90",
            LightRight => "map.light_right",
            LightLeft => "map.light_left",
            Pan => "map.pan",
            SelectAll => "select.all",
            Deselect => "select.none",
            InvertSelection => "select.invert",
            SelectAdd => "select.add",
            SelectSubtract => "select.subtract",
            MapZoomIn => "map.zoom_in",
            MapZoomOut => "map.zoom_out",
            MapZoomReset => "map.zoom_reset",
            FlyForward => "fly.forward",
            FlyBack => "fly.back",
            FlyLeft => "fly.left",
            FlyRight => "fly.right",
            FlyDown => "fly.down",
            FlyUp => "fly.up",
            FlyFaster => "fly.faster",
            WalkForward => "player.forward",
            WalkBack => "player.back",
            WalkLeft => "player.left",
            WalkRight => "player.right",
            Jump => "player.jump",
            Sneak => "player.sneak",
            Sprint => "player.sprint",
            LookLeft => "player.look_left",
            LookRight => "player.look_right",
            LookUp => "player.look_up",
            LookDown => "player.look_down",
            ReleaseMouse => "player.release_mouse",
        };
        s.to_owned()
    }

    pub fn area(self) -> Area {
        use Action::*;
        match self {
            NewWorld | Open | Save | SaveAs | Export | Undo | Redo | Settings | Help | UiZoomIn | UiZoomOut | UiZoomReset | MaximizeArea | Delete | Cancel => Area::General,
            Tool(_) => Area::Tools,
            Intensity(_) | BrushSmaller | BrushLarger | TurnLeft | TurnRight | TurnReset | Turn30 | Turn45 | Turn90 => Area::Brushes,
            LightRight | LightLeft | Pan | SelectAll | Deselect | InvertSelection | SelectAdd | SelectSubtract | MapZoomIn | MapZoomOut | MapZoomReset => Area::Map,
            FlyForward | FlyBack | FlyLeft | FlyRight | FlyDown | FlyUp | FlyFaster => Area::View3d,
            WalkForward | WalkBack | WalkLeft | WalkRight | Jump | Sneak | Sprint | LookLeft | LookRight | LookUp | LookDown | ReleaseMouse => Area::Player,
        }
    }

    pub fn scope(self) -> Scope {
        use Action::*;
        match self {
            Pan | SelectAdd | SelectSubtract | Cancel => Scope::Map,
            _ => match self.area() {
                Area::View3d => Scope::Fly,
                Area::Player => Scope::Player,
                _ => Scope::Global,
            },
        }
    }

    pub fn kind(self) -> Kind {
        use Action::*;
        match self {
            SelectAdd | SelectSubtract => Kind::Modifier,
            Pan | FlyForward | FlyBack | FlyLeft | FlyRight | FlyDown | FlyUp | FlyFaster | WalkForward | WalkBack | WalkLeft | WalkRight | Jump | Sneak | Sprint | LookLeft | LookRight | LookUp | LookDown => Kind::Hold,
            _ => Kind::Press,
        }
    }

    /// Done even while a text field is typed in, when its keys hold ⌘
    /// (Ctrl): typing has no use for them.
    fn while_typing(self) -> bool {
        use Action::*;
        matches!(self, NewWorld | Open | Save | SaveAs | Export | Settings | Help | UiZoomIn | UiZoomOut | UiZoomReset)
    }

    /// What it does, as the settings list it.
    pub fn name(self) -> String {
        use Action::*;
        let s = match self {
            Tool(tool) => return tool.label().to_owned(),
            Intensity(n) => return tr!("Intensity {n} %", n = n as u32 * 10),
            NewWorld => tr!("New world"),
            Open => tr!("Open"),
            Save => tr!("Save"),
            SaveAs => tr!("Save as"),
            Export => tr!("Export"),
            Undo => tr!("Undo"),
            Redo => tr!("Redo"),
            Settings => tr!("Settings"),
            Help => tr!("Help"),
            UiZoomIn => tr!("Larger interface"),
            UiZoomOut => tr!("Smaller interface"),
            UiZoomReset => tr!("Interface at its size"),
            MaximizeArea => tr!("Fill the window with the area"),
            Delete => tr!("Delete the point or vein"),
            Cancel => tr!("Cancel the stroke or selection"),
            BrushSmaller => tr!("Smaller brush"),
            BrushLarger => tr!("Larger brush"),
            TurnLeft => tr!("Turn left 15°"),
            TurnRight => tr!("Turn right 15°"),
            TurnReset => tr!("Turn back straight"),
            Turn30 => tr!("Turn 30°"),
            Turn45 => tr!("Turn 45°"),
            Turn90 => tr!("Turn 90°"),
            LightRight => tr!("Turn the light right"),
            LightLeft => tr!("Turn the light left"),
            Pan => tr!("Move the map (hold, drag)"),
            SelectAll => tr!("Select all"),
            Deselect => tr!("Deselect"),
            InvertSelection => tr!("Invert the selection"),
            SelectAdd => tr!("Add to the selection (hold)"),
            SelectSubtract => tr!("Take out of the selection (hold)"),
            MapZoomIn => tr!("Zoom in"),
            MapZoomOut => tr!("Zoom out"),
            MapZoomReset => tr!("Zoom to 100 %"),
            FlyForward => tr!("Forward"),
            FlyBack => tr!("Back"),
            FlyLeft => tr!("Left"),
            FlyRight => tr!("Right"),
            FlyDown => tr!("Down"),
            FlyUp => tr!("Up"),
            FlyFaster => tr!("Faster (hold)"),
            WalkForward => tr!("Walk forward"),
            WalkBack => tr!("Walk back"),
            WalkLeft => tr!("Walk left"),
            WalkRight => tr!("Walk right"),
            Jump => tr!("Jump, fly up"),
            Sneak => tr!("Sneak, fly down"),
            Sprint => tr!("Sprint"),
            LookLeft => tr!("Look left"),
            LookRight => tr!("Look right"),
            LookUp => tr!("Look up"),
            LookDown => tr!("Look down"),
            ReleaseMouse => tr!("Give the mouse back"),
        };
        s.to_owned()
    }

    /// Its keys in `preset`; the first is the one menus show.
    pub fn defaults(self, preset: Preset) -> Vec<Chord> {
        self.preset_keys(preset).into_iter().map(|c| Chord { mods: c.mods.normal(), ..c }).collect()
    }

    fn preset_keys(self, preset: Preset) -> Vec<Chord> {
        use Action::*;
        let c = |key| Chord::with(Mods::COMMAND, key);
        let k = Chord::key;
        let alt = |key| Chord::with(Mods::ALT, key);
        let wp = preset == Preset::WorldPainter;
        match self {
            NewWorld => vec![c(Key::N)],
            Open => vec![c(Key::O)],
            Save => vec![c(Key::S)],
            SaveAs => vec![Chord::with(Mods::COMMAND_SHIFT, Key::S)],
            Export => vec![c(Key::E)],
            Undo => vec![c(Key::Z)],
            // WorldPainter's redo is ⌘Y first.
            Redo if wp => vec![c(Key::Y), Chord::with(Mods::COMMAND_SHIFT, Key::Z)],
            Redo => vec![Chord::with(Mods::COMMAND_SHIFT, Key::Z), c(Key::Y)],
            Settings => vec![c(Key::Comma)],
            Help => vec![k(Key::F1)],
            // WorldPainter zooms the map with ⌘ and +, − and 0; Massif the
            // interface, as a browser does (Settings ▸ Accessibility).
            UiZoomIn if !wp => vec![c(Key::Plus), c(Key::Equals)],
            UiZoomOut if !wp => vec![c(Key::Minus)],
            UiZoomReset if !wp => vec![c(Key::Num0)],
            MapZoomIn if wp => vec![c(Key::Plus), c(Key::Equals)],
            MapZoomOut if wp => vec![c(Key::Minus)],
            MapZoomReset if wp => vec![c(Key::Num0)],
            UiZoomIn | UiZoomOut | UiZoomReset | MapZoomIn | MapZoomOut | MapZoomReset => vec![],
            MaximizeArea => vec![Chord::with(Mods::CTRL, Key::Space)],
            Delete => vec![k(Key::Delete), k(Key::Backspace)],
            Cancel => vec![k(Key::Escape)],
            Tool(tool) => match tool {
                crate::tools::Tool::Raise => vec![alt(Key::H)],
                crate::tools::Tool::Flatten => vec![alt(Key::A)],
                crate::tools::Tool::Smooth => vec![alt(Key::S)],
                crate::tools::Tool::PaintTerrain => vec![alt(Key::P), alt(Key::R)],
                crate::tools::Tool::Eyedropper => vec![alt(Key::Y)],
                crate::tools::Tool::Flood => vec![alt(Key::F)],
                _ => vec![],
            },
            Intensity(n) => {
                const DIGITS: [Key; 10] = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9, Key::Num0];
                DIGITS.get(n as usize - 1).map(|d| vec![k(*d)]).unwrap_or_default()
            }
            BrushSmaller => vec![k(Key::Minus)],
            BrushLarger => vec![k(Key::Plus), k(Key::Equals)],
            TurnLeft => vec![k(Key::OpenBracket), alt(Key::Minus)],
            TurnRight => vec![k(Key::CloseBracket), alt(Key::Plus), alt(Key::Equals)],
            TurnReset => vec![alt(Key::Num0)],
            Turn30 => vec![alt(Key::Num3)],
            Turn45 => vec![alt(Key::Num4)],
            Turn90 => vec![alt(Key::Num9)],
            LightRight => vec![k(Key::R)],
            LightLeft => vec![k(Key::L)],
            Pan => vec![k(Key::Space)],
            SelectAll => vec![c(Key::A)],
            Deselect => vec![c(Key::D)],
            InvertSelection => vec![Chord::with(Mods::COMMAND_SHIFT, Key::I)],
            SelectAdd => vec![Chord::mods(Mods::SHIFT), Chord::mods(Mods::COMMAND)],
            SelectSubtract => vec![Chord::mods(Mods::ALT)],
            FlyForward => vec![k(Key::W), k(Key::ArrowUp)],
            FlyBack => vec![k(Key::S), k(Key::ArrowDown)],
            FlyLeft => vec![k(Key::A), k(Key::ArrowLeft)],
            FlyRight => vec![k(Key::D), k(Key::ArrowRight)],
            FlyDown => vec![k(Key::Q), k(Key::PageDown)],
            FlyUp => vec![k(Key::E), k(Key::PageUp)],
            FlyFaster => vec![Chord::mods(Mods::SHIFT)],
            WalkForward => vec![k(Key::W)],
            WalkBack => vec![k(Key::S)],
            WalkLeft => vec![k(Key::A)],
            WalkRight => vec![k(Key::D)],
            Jump => vec![k(Key::Space)],
            Sneak => vec![Chord::mods(Mods::SHIFT), k(Key::Q)],
            Sprint => vec![Chord::mods(Mods::CTRL)],
            LookLeft => vec![k(Key::ArrowLeft)],
            LookRight => vec![k(Key::ArrowRight)],
            LookUp => vec![k(Key::ArrowUp)],
            LookDown => vec![k(Key::ArrowDown)],
            ReleaseMouse => vec![k(Key::Escape)],
        }
    }

    /// Whether `chord` can be one of its keys: a modifier held while the
    /// mouse acts is a modifier alone; what is pressed needs a key.
    pub fn accepts(self, chord: &Chord) -> bool {
        match self.kind() {
            Kind::Press => chord.key.is_some(),
            Kind::Hold => true,
            Kind::Modifier => chord.key.is_none() && !chord.mods.is_none(),
        }
    }
}

/// A set of keys to start from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Preset {
    #[default]
    Massif,
    /// WorldPainter's own keys, as its App.java binds them.
    WorldPainter,
}

impl Preset {
    pub const ALL: [Preset; 2] = [Preset::Massif, Preset::WorldPainter];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Massif => "Massif",
            Preset::WorldPainter => "WorldPainter",
        }
    }
}

/// The keys as the settings keep them: a preset, and what the user changed
/// in it, action by action (by id, each shortcut as `Chord::id` writes it;
/// no shortcut at all is an empty list).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub preset: Preset,
    pub changed: BTreeMap<String, Vec<String>>,
}

/// A keymap file, as Settings ▸ Keyboard exports and imports it: every
/// action's keys, not only the changes, so that it reads on its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct File {
    pub massif_keymap: u32,
    #[serde(default)]
    pub preset: Preset,
    #[serde(default)]
    pub keys: BTreeMap<String, Vec<String>>,
}

impl Saved {
    /// The keymap these settings make.
    pub fn keymap(&self) -> Keymap {
        let mut keys = Vec::new();
        for action in Action::all() {
            let chords = match self.changed.get(&action.id()) {
                Some(list) => {
                    let mut out: Vec<Chord> = Vec::new();
                    for c in list.iter().filter_map(|s| Chord::parse(s)).filter(|c| action.accepts(c)) {
                        if !out.contains(&c) {
                            out.push(c);
                        }
                    }
                    out
                }
                None => action.defaults(self.preset),
            };
            keys.push((action, chords));
        }
        Keymap { preset: self.preset, keys }
    }

    /// Keep `keys` for `action`: as a change, or none when they are the
    /// preset's.
    pub fn set(&mut self, action: Action, keys: &[Chord]) {
        if keys == action.defaults(self.preset).as_slice() {
            self.changed.remove(&action.id());
        } else {
            self.changed.insert(action.id(), keys.iter().map(Chord::id).collect());
        }
    }

    /// The file `export` writes: the preset, and every action's keys.
    pub fn export(&self) -> String {
        let map = self.keymap();
        let keys = map.keys.iter().map(|(a, chords)| (a.id(), chords.iter().map(Chord::id).collect())).collect();
        serde_json::to_string_pretty(&File { massif_keymap: 1, preset: self.preset, keys }).unwrap_or_default()
    }

    /// The settings a keymap file makes, or why it does not read. Ids this
    /// version does not know are left aside; actions the file does not name
    /// keep the preset's keys.
    pub fn import(text: &str) -> Result<Saved, String> {
        let file: File = serde_json::from_str(text).map_err(|e| tr!("Not a Massif keymap: {e}", e))?;
        let mut saved = Saved { preset: file.preset, changed: BTreeMap::new() };
        let known: Vec<Action> = Action::all();
        for action in known {
            if let Some(list) = file.keys.get(&action.id()) {
                let chords: Vec<Chord> = list.iter().filter_map(|s| Chord::parse(s)).filter(|c| action.accepts(c)).collect();
                saved.set(action, &chords);
            }
        }
        Ok(saved)
    }
}

/// Every action's keys.
#[derive(Debug, Clone, PartialEq)]
pub struct Keymap {
    pub preset: Preset,
    keys: Vec<(Action, Vec<Chord>)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Saved::default().keymap()
    }
}

impl Keymap {
    pub fn keys(&self, action: Action) -> &[Chord] {
        self.keys.iter().find(|(a, _)| *a == action).map(|(_, c)| c.as_slice()).unwrap_or(&[])
    }

    /// The first of its keys, as menus show it.
    pub fn shortcut(&self, action: Action) -> Option<KeyboardShortcut> {
        self.keys(action).iter().find_map(Chord::shortcut)
    }

    /// Its first key, as text (⌥H, Alt+H), for tips.
    pub fn label(&self, action: Action) -> Option<String> {
        self.keys(action).first().map(Chord::label)
    }

    /// The other actions `chord` is a key of where `action` listens.
    pub fn clashes(&self, action: Action, chord: &Chord) -> Vec<Action> {
        self.keys.iter().filter(|(a, chords)| *a != action && a.scope().meets(action.scope()) && chords.contains(chord)).map(|(a, _)| *a).collect()
    }

    /// The actions a key press does: those bound exactly to it, else to it
    /// without Shift, else the same by the key's place.
    fn matches(&self, key: Key, physical: Option<Key>, mods: Mods, typing: bool) -> Vec<(Action, bool)> {
        let candidates = || self.keys.iter().filter(|(a, _)| a.kind() == Kind::Press && (!typing || a.while_typing()));
        for k in std::iter::once(key).chain(physical.filter(|p| *p != key)) {
            for loose in [false, true] {
                if loose && !mods.shift {
                    continue;
                }
                let want = if loose { Mods { shift: false, ..mods } } else { mods };
                let found: Vec<(Action, bool)> = candidates()
                    .filter(|(a, chords)| chords.iter().any(|c| c.key == Some(k) && c.mods == want && (!typing || c.mods.command || c.mods.ctrl || is_function(k) || !a.while_typing())))
                    .map(|(a, _)| (*a, loose))
                    .collect();
                if !found.is_empty() {
                    return found;
                }
            }
        }
        Vec::new()
    }
}

fn is_function(key: Key) -> bool {
    matches!(key, Key::F1 | Key::F2 | Key::F3 | Key::F4 | Key::F5 | Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::F11 | Key::F12)
}

/// A press of an action's keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    /// Shift was held besides its keys: a finer step (a block at a time).
    pub shift: bool,
}

#[derive(Default)]
struct Frame {
    pass: Option<u64>,
    pressed: Vec<(Action, bool)>,
}

thread_local! {
    // The app's frames run on one thread, and each test on its own: each
    // has its keys.
    static CURRENT: RefCell<Rc<Keymap>> = RefCell::new(Rc::new(Keymap::default()));
    static FRAME: RefCell<Frame> = RefCell::new(Frame::default());
    /// The frame in which Settings ▸ Keyboard last listened for new keys:
    /// while it does, none does anything.
    static RECORDING: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    /// The frame now, as `apply` was told.
    static PASS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// The settings `CURRENT` was made from.
    static APPLIED: RefCell<Option<Saved>> = const { RefCell::new(None) };
}

/// Use the keys `saved` makes, made again only when they changed: called
/// once a frame, before anything reads the keys.
pub fn apply(ctx: &egui::Context, saved: &Saved) {
    PASS.with(|p| p.set(ctx.cumulative_pass_nr()));
    let same = APPLIED.with(|a| a.borrow().as_ref() == Some(saved));
    if !same {
        set(saved.keymap());
        APPLIED.with(|a| *a.borrow_mut() = Some(saved.clone()));
    }
}

/// The keymap in use.
pub fn get() -> Rc<Keymap> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Use `map` from now on.
pub fn set(map: Keymap) {
    CURRENT.with(|c| {
        if **c.borrow() != map {
            *c.borrow_mut() = Rc::new(map);
        }
    });
}

/// Keys are being recorded this frame: until the frame after the last
/// that says so, keys pressed do nothing.
pub fn set_recording() {
    RECORDING.with(|r| r.set(Some(PASS.with(|p| p.get()))));
}

pub fn recording() -> bool {
    let now = PASS.with(|p| p.get());
    RECORDING.with(|r| r.get()).is_some_and(|p| p + 1 >= now)
}

/// Whether `action`'s keys were pressed this frame, and how; each press is
/// taken once.
pub fn take(ctx: &egui::Context, action: Action) -> Option<Press> {
    let pass = ctx.cumulative_pass_nr();
    FRAME.with(|f| {
        let mut f = f.borrow_mut();
        if f.pass != Some(pass) {
            f.pass = Some(pass);
            f.pressed = if recording() { Vec::new() } else { resolve(ctx) };
        }
        let i = f.pressed.iter().position(|(a, _)| *a == action)?;
        let (_, shift) = f.pressed.remove(i);
        Some(Press { shift })
    })
}

/// Whether `action`'s keys were pressed this frame, without taking them.
pub fn peek(ctx: &egui::Context, action: Action) -> bool {
    let pass = ctx.cumulative_pass_nr();
    let fresh = FRAME.with(|f| f.borrow().pass == Some(pass));
    if !fresh {
        // Read the frame's presses, then put back what was taken.
        if let Some(p) = take(ctx, action) {
            FRAME.with(|f| f.borrow_mut().pressed.push((action, p.shift)));
            return true;
        }
        return false;
    }
    FRAME.with(|f| f.borrow().pressed.iter().any(|(a, _)| *a == action))
}

fn resolve(ctx: &egui::Context) -> Vec<(Action, bool)> {
    let typing = ctx.egui_wants_keyboard_input();
    let map = get();
    ctx.input(|i| {
        let mut out = Vec::new();
        for e in &i.events {
            if let egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } = e {
                out.extend(map.matches(*key, *physical_key, Mods::of(*modifiers), typing));
            }
        }
        out
    })
}

/// Whether `action`'s keys are held down now: a key (with at least its
/// modifiers), or its modifiers alone.
pub fn held(i: &egui::InputState, action: Action) -> bool {
    if recording() {
        return false;
    }
    let mods = Mods::of(i.modifiers);
    get().keys(action).iter().any(|c| {
        c.mods.within(mods)
            && match c.key {
                Some(k) => i.key_down(k),
                None => !c.mods.is_none(),
            }
    })
}

/// Whether one of `action`'s keys went down this frame (not repeated by the
/// system).
pub fn first_press(i: &egui::InputState, action: Action) -> bool {
    if recording() {
        return false;
    }
    let map = get();
    let keys = map.keys(action);
    i.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, repeat: false, .. } if keys.iter().any(|c| c.key == Some(*key))))
}

/// Whether the modifiers held, `m`, hold one of `action`'s.
pub fn mods_hold(m: Modifiers, action: Action) -> bool {
    let mods = Mods::of(m);
    get().keys(action).iter().any(|c| c.key.is_none() && !c.mods.is_none() && c.mods.within(mods))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(map: &Keymap, mods: Mods, key: Key, typing: bool) -> Vec<(Action, bool)> {
        map.matches(key, None, mods, typing)
    }

    /// The keys are the ones Massif always had (docs/guide.md, Controls).
    #[test]
    fn defaults_unchanged() {
        let map = Keymap::default();
        let ids = |a: Action| map.keys(a).iter().map(Chord::id).collect::<Vec<_>>();
        assert_eq!(ids(Action::Save), ["Cmd+S"]);
        assert_eq!(ids(Action::SaveAs), ["Shift+Cmd+S"]);
        assert_eq!(ids(Action::Redo), ["Shift+Cmd+Z", "Cmd+Y"]);
        assert_eq!(ids(Action::Help), ["F1"]);
        assert_eq!(ids(Action::Tool(Tool::Raise)), ["Alt+H"]);
        assert_eq!(ids(Action::Tool(Tool::PaintTerrain)), ["Alt+P", "Alt+R"]);
        assert!(ids(Action::Tool(Tool::Roughen)).is_empty());
        assert_eq!(ids(Action::Intensity(3)), ["3"]);
        assert_eq!(ids(Action::Intensity(10)), ["0"]);
        assert_eq!(ids(Action::TurnLeft), ["OpenBracket", "Alt+Minus"]);
        assert_eq!(ids(Action::LightRight), ["R"]);
        assert_eq!(ids(Action::Pan), ["Space"]);
        assert_eq!(ids(Action::SelectAdd), ["Shift", "Cmd"]);
        assert_eq!(ids(Action::FlyUp), ["E", "PageUp"]);
        assert_eq!(ids(Action::Sneak), ["Shift", "Q"]);
        if MAC {
            assert_eq!(ids(Action::MaximizeArea), ["Ctrl+Space"]);
            assert_eq!(ids(Action::Sprint), ["Ctrl"]);
        } else {
            assert_eq!(ids(Action::MaximizeArea), ["Cmd+Space"]);
        }
        // Every action has its own id, and no two share keys where both
        // listen, in either preset.
        let all = Action::all();
        let mut seen: Vec<String> = all.iter().map(|a| a.id()).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), all.len());
        for preset in Preset::ALL {
            let map = Saved { preset, ..Saved::default() }.keymap();
            for a in &all {
                for c in map.keys(*a) {
                    assert!(map.clashes(*a, c).is_empty(), "{preset:?}: {} on {:?} and {:?}", c.id(), a, map.clashes(*a, c));
                    assert!(a.accepts(c), "{a:?} takes {}", c.id());
                }
            }
        }
        // WorldPainter's: ⌘Y first for redo, ⌘ + − 0 zoom the map.
        let wp = Saved { preset: Preset::WorldPainter, ..Saved::default() }.keymap();
        assert_eq!(wp.keys(Action::Redo)[0].id(), "Cmd+Y");
        assert_eq!(wp.keys(Action::MapZoomIn)[0].id(), "Cmd+Plus");
        assert!(wp.keys(Action::UiZoomIn).is_empty());
    }

    #[test]
    fn chords_read_back() {
        for key in Key::ALL {
            for mods in [Mods::NONE, Mods::COMMAND_SHIFT, Mods { alt: true, ctrl: true, ..Mods::NONE }] {
                let c = Chord::with(mods.normal(), *key);
                assert_eq!(Chord::parse(&c.id()), Some(c), "{}", c.id());
            }
        }
        assert_eq!(Chord::parse("Shift"), Some(Chord::mods(Mods::SHIFT)));
        assert_eq!(Chord::parse("cmd+shift+z"), Some(Chord::with(Mods::COMMAND_SHIFT, Key::Z)));
        assert_eq!(Chord::parse("Cmd+Nonsense"), None);
        assert_eq!(Chord::parse(""), None);
        assert_eq!(Chord::parse("A+B"), None);
    }

    #[test]
    fn labels_per_system() {
        let redo = Chord::with(Mods::COMMAND_SHIFT, Key::Z);
        assert_eq!(redo.label_for(true), "⇧⌘Z");
        assert_eq!(redo.label_for(false), "Ctrl+Shift+Z");
        assert_eq!(Chord::with(Mods::ALT, Key::H).label_for(true), "⌥H");
        assert_eq!(Chord::with(Mods::ALT, Key::H).label_for(false), "Alt+H");
        assert_eq!(Chord::mods(Mods::SHIFT).label_for(true), "Shift");
        assert_eq!(Chord::key(Key::Minus).label_for(false), "−");
        assert_eq!(Chord::with(Mods::CTRL, Key::Space).label_for(true), "⌃Space");
    }

    #[test]
    fn presses_match_the_closest_binding() {
        let map = Keymap::default();
        assert_eq!(press(&map, Mods::COMMAND_SHIFT, Key::Z, false), [(Action::Redo, false)]);
        assert_eq!(press(&map, Mods::COMMAND, Key::Z, false), [(Action::Undo, false)]);
        // Shift with − makes the brush a block smaller.
        assert_eq!(press(&map, Mods::SHIFT, Key::Minus, false), [(Action::BrushSmaller, true)]);
        // Escape cancels on the map and gives the mouse back in Player.
        assert_eq!(press(&map, Mods::NONE, Key::Escape, false), [(Action::Cancel, false), (Action::ReleaseMouse, false)]);
        // By the key's place: an AZERTY 3 types ", its place is 3.
        assert_eq!(map.matches(Key::Quote, Some(Key::Num3), Mods::NONE, false), [(Action::Intensity(3), false)]);
        // Typing: only what typing has no use for.
        assert!(press(&map, Mods::NONE, Key::Num3, true).is_empty());
        assert!(press(&map, Mods::COMMAND, Key::Z, true).is_empty());
        assert!(press(&map, Mods::ALT, Key::H, true).is_empty());
        assert!(press(&map, Mods::NONE, Key::Delete, true).is_empty());
        assert_eq!(press(&map, Mods::COMMAND, Key::S, true), [(Action::Save, false)]);
        assert_eq!(press(&map, Mods::NONE, Key::F1, true), [(Action::Help, false)]);
    }

    #[test]
    fn rebinding_and_clashes() {
        let mut saved = Saved::default();
        // Raise / Lower on Alt+R, which Paint terrain has.
        let alt_r = Chord::with(Mods::ALT, Key::R);
        assert_eq!(saved.keymap().clashes(Action::Tool(Tool::Raise), &alt_r), [Action::Tool(Tool::PaintTerrain)]);
        // W is Forward in the 3D view and in Player, which never listen at
        // once; on a map tool it clashes with both.
        let w = Chord::key(Key::W);
        assert!(saved.keymap().clashes(Action::FlyForward, &w).is_empty());
        assert_eq!(saved.keymap().clashes(Action::Tool(Tool::Road), &w), [Action::FlyForward, Action::WalkForward]);
        // Replace: Paint terrain loses it, Raise takes it.
        saved.set(Action::Tool(Tool::PaintTerrain), &[Chord::with(Mods::ALT, Key::P)]);
        saved.set(Action::Tool(Tool::Raise), &[Chord::with(Mods::ALT, Key::H), alt_r]);
        let map = saved.keymap();
        assert_eq!(press(&map, Mods::ALT, Key::R, false), [(Action::Tool(Tool::Raise), false)]);
        assert_eq!(saved.changed.len(), 2);
        // Set back to the preset's, a change is no longer kept.
        saved.set(Action::Tool(Tool::Raise), &Action::Tool(Tool::Raise).defaults(Preset::Massif));
        assert_eq!(saved.changed.len(), 1);
        // No key at all is kept as such.
        saved.set(Action::LightLeft, &[]);
        assert!(saved.keymap().keys(Action::LightLeft).is_empty());
        assert!(press(&saved.keymap(), Mods::NONE, Key::L, false).is_empty());
        // What an action cannot take is left out: Pan held with a key, a
        // modifier alone for Save.
        saved.changed.insert(Action::Save.id(), vec!["Shift".into(), "Cmd+K".into()]);
        assert_eq!(saved.keymap().keys(Action::Save), [Chord::with(Mods::COMMAND, Key::K)]);
    }

    /// Saved keys survive updates: an id this version does not know is left
    /// aside, an action it does not name keeps its preset's keys.
    #[test]
    fn keymap_files_round_trip() {
        let mut saved = Saved { preset: Preset::WorldPainter, ..Saved::default() };
        saved.set(Action::Intensity(5), &[Chord::key(Key::K)]);
        saved.set(Action::Jump, &[Chord::key(Key::Space), Chord::mods(Mods::ALT)]);
        let text = saved.export();
        assert!(text.contains("\"brush.intensity_50\""), "{text}");
        assert_eq!(Saved::import(&text).unwrap(), saved);
        let old = r#"{"massif_keymap": 1, "keys": {"brush.intensity_50": ["K"], "tool.gone": ["Alt+Q"], "file.save": ["Cmd+Bogus", "F5"]}}"#;
        let imported = Saved::import(old).unwrap();
        let map = imported.keymap();
        assert_eq!(map.keys(Action::Intensity(5)), [Chord::key(Key::K)]);
        assert_eq!(map.keys(Action::Save), [Chord::key(Key::F5)]);
        assert_eq!(map.keys(Action::Open), Action::Open.defaults(Preset::Massif).as_slice());
        assert!(!imported.changed.contains_key("tool.gone"));
        assert!(Saved::import("{\"keys\": 3}").is_err());
        // The settings, as stored, read the same way.
        let json = r#"{"preset": "Massif", "changed": {"edit.undo": ["Cmd+U"], "some.future.action": ["X"]}}"#;
        let stored: Saved = serde_json::from_str(json).unwrap();
        assert_eq!(stored.keymap().keys(Action::Undo), [Chord::with(Mods::COMMAND, Key::U)]);
    }
}
