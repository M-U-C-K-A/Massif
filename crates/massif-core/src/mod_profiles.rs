//! What Massif knows of block mods beyond what their jars say, as data:
//! the profiles of `assets/mods/<namespace>/`, bundled when Massif is built
//! (see `assets/mods/README.md`). No mod is known in code: a profile only
//! comes into play once its mod's blocks are read, through the same paths
//! as any mod's.

use crate::materials::Palette;
use serde::Deserialize;
use std::sync::OnceLock;

/// A block mod's profile.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ModProfile {
    /// The mod's name, as players know it.
    pub name: String,
    /// The namespace of its blocks (`conquest` of `conquest:old_bricks`).
    pub namespace: String,
    /// The version of the game the mod is made for, when it is made for
    /// one.
    #[serde(default)]
    pub minecraft: Option<String>,
    /// A palette of its blocks, offered once the mod is read.
    #[serde(skip)]
    pub palette: Option<Palette>,
}

/// The bundled files: (profile.json, palette.json), one per mod.
const FILES: &[(&str, Option<&str>)] = include!(concat!(env!("OUT_DIR"), "/mod_profiles.rs"));

/// Every bundled profile, by namespace.
pub fn all() -> &'static [ModProfile] {
    static ALL: OnceLock<Vec<ModProfile>> = OnceLock::new();
    ALL.get_or_init(|| {
        FILES
            .iter()
            .map(|(profile, palette)| {
                let mut p: ModProfile = serde_json::from_str(profile).expect("a mod's profile.json");
                p.palette = palette.map(|text| serde_json::from_str(text).expect("a mod's palette.json"));
                p
            })
            .collect()
    })
}

/// The profile of the mod whose blocks are in `namespace`.
pub fn of(namespace: &str) -> Option<&'static ModProfile> {
    all().iter().find(|p| p.namespace == namespace)
}

/// The palettes of the mods whose blocks are read (`read`: their
/// namespaces): each mod's, when its profile has one and every block it
/// uses is of a mod read.
pub fn palettes_of(read: &[&str]) -> Vec<Palette> {
    all().iter().filter_map(|p| p.palette.as_ref()).filter(|p| p.namespaces().iter().all(|ns| read.contains(ns))).cloned().collect()
}

/// The version of the game a palette's blocks are for: the one the
/// profile of a mod it uses names, if any.
pub fn minecraft_of(palette: &Palette) -> Option<&'static str> {
    palette.namespaces().iter().find_map(|ns| of(ns)?.minecraft.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_read() {
        assert!(!all().is_empty());
        let mut seen = Vec::new();
        for p in all() {
            assert!(!p.name.is_empty() && !p.namespace.is_empty() && p.namespace != "minecraft", "{p:?}");
            assert!(!seen.contains(&&p.namespace), "{} twice", p.namespace);
            seen.push(&p.namespace);
            if let Some(palette) = &p.palette {
                assert_eq!(palette.namespaces(), [p.namespace.as_str()], "{}: its palette is of its own blocks", p.name);
                assert!(!palette.is_vanilla());
                assert_eq!(minecraft_of(palette), p.minecraft.as_deref());
                assert_eq!(palettes_of(&[]), Vec::<Palette>::new(), "nothing offered while no mod is read");
                assert!(palettes_of(&[&p.namespace]).contains(palette));
            }
        }
        assert_eq!(minecraft_of(&Palette::vanilla()), None);
    }
}
