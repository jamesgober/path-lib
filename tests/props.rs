//! Property tests: invariants that must hold for every possible input.

#![allow(clippy::unwrap_used, missing_docs)]

use std::path::{Component, Path, PathBuf};

use proptest::prelude::*;

fn base() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\base\dir")
    } else {
        PathBuf::from("/base/dir")
    }
}

/// Strings biased toward the characters that matter: separators, dots,
/// reserved punctuation, controls, and assorted Unicode.
fn nasty_string() -> impl Strategy<Value = String> {
    let pieces = prop_oneof![
        Just("/".to_owned()),
        Just("\\".to_owned()),
        Just(".".to_owned()),
        Just("..".to_owned()),
        Just("~".to_owned()),
        Just(":".to_owned()),
        Just("C:".to_owned()),
        Just("CON".to_owned()),
        Just(" ".to_owned()),
        Just("\0".to_owned()),
        Just("\u{202E}".to_owned()),
        Just("\u{FF0F}".to_owned()),
        Just("名".to_owned()),
        "[a-zA-Z0-9_-]{1,8}",
        any::<char>().prop_map(String::from),
    ];
    prop::collection::vec(pieces, 0..24).prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// `join` never panics, and when it succeeds the result is inside the
    /// base with only plain named components after it.
    #[test]
    fn prop_join_is_confined(sub in nasty_string()) {
        let base = base();
        if let Ok(p) = path_lib::join(&base, &sub) {
            prop_assert!(p.starts_with(&base));
            let rest = p.strip_prefix(&base).unwrap();
            for c in rest.components() {
                prop_assert!(matches!(c, Component::Normal(_)), "{c:?} in {p:?}");
            }
        }
    }

    /// Whatever `sanitize` produces is always accepted by `join` as exactly
    /// one new segment.
    #[test]
    fn prop_sanitize_output_is_always_one_valid_segment(input in nasty_string()) {
        let name = path_lib::sanitize(&input);
        prop_assert!(!name.is_empty());
        prop_assert!(name.len() <= path_lib::MAX_SEGMENT_BYTES);
        let base = base();
        let p = path_lib::join(&base, &name).unwrap();
        prop_assert_eq!(p.parent(), Some(base.as_path()));
    }

    /// `sanitize` is idempotent.
    #[test]
    fn prop_sanitize_is_idempotent(input in nasty_string()) {
        let once = path_lib::sanitize(&input);
        prop_assert_eq!(path_lib::sanitize(&once), once);
    }

    /// `clean` is idempotent and never leaves `.` components.
    #[test]
    fn prop_clean_is_idempotent(input in nasty_string()) {
        let once = path_lib::clean(Path::new(&input));
        prop_assert_eq!(path_lib::clean(&once), once.clone());
        let curdirs = once.components().filter(|c| matches!(c, Component::CurDir)).count();
        prop_assert!(curdirs == 0 || once == Path::new("."));
    }

    /// Joining is deterministic regardless of which slash style is used.
    #[test]
    fn prop_join_slash_style_is_irrelevant(segs in prop::collection::vec("[a-z0-9]{1,6}", 0..10)) {
        let fwd = segs.join("/");
        let back = segs.join("\\");
        prop_assert_eq!(
            path_lib::join(base(), &fwd).unwrap(),
            path_lib::join(base(), &back).unwrap()
        );
    }
}
