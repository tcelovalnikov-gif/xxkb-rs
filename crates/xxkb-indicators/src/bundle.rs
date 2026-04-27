//! Compiled-in fallback flags.
//!
//! These are simplified, license-clean SVGs shipped with the daemon so
//! a fresh install always renders something, even before the user
//! installs system flag packs.

const ICONS: &[(&str, &str, &[u8])] = &[
    (
        "en",
        "en.svg",
        include_bytes!("../../../assets/icons/en.svg"),
    ),
    (
        "ru",
        "ru.svg",
        include_bytes!("../../../assets/icons/ru.svg"),
    ),
    (
        "ua",
        "ua.svg",
        include_bytes!("../../../assets/icons/ua.svg"),
    ),
    (
        "by",
        "by.svg",
        include_bytes!("../../../assets/icons/by.svg"),
    ),
    (
        "kz",
        "kz.svg",
        include_bytes!("../../../assets/icons/kz.svg"),
    ),
    (
        "de",
        "de.svg",
        include_bytes!("../../../assets/icons/de.svg"),
    ),
    (
        "fr",
        "fr.svg",
        include_bytes!("../../../assets/icons/fr.svg"),
    ),
];

/// Look up a builtin icon by logical name (e.g. `"en"`, `"ru"`).
///
/// Returns `(stable_id, bytes)` on hit. The id is the file name with
/// extension and is stable across releases — used as a cache key.
#[must_use]
pub fn lookup(name: &str) -> Option<(&'static str, &'static [u8])> {
    ICONS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, id, bytes)| (*id, *bytes))
}

/// Iterate over every builtin icon. Useful for the configurator's icon
/// picker.
pub fn all() -> impl Iterator<Item = (&'static str, &'static str, &'static [u8])> {
    ICONS.iter().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_resolves() {
        for (name, id, bytes) in all() {
            let (id2, bytes2) = lookup(name).unwrap();
            assert_eq!(id, id2);
            assert_eq!(bytes, bytes2);
            assert!(!bytes.is_empty(), "{name} bytes are empty");
            // Each builtin is an SVG.
            assert!(id.ends_with(".svg"));
        }
    }

    #[test]
    fn unknown_returns_none() {
        assert!(lookup("zz").is_none());
    }

    #[test]
    fn full_set_present() {
        let names: Vec<_> = all().map(|(n, _, _)| n).collect();
        for expected in ["en", "ru", "ua", "by", "kz", "de", "fr"] {
            assert!(names.contains(&expected), "missing builtin: {expected}");
        }
    }
}
