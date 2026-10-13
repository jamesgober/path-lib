//! Parser for the freedesktop `user-dirs.dirs` file.
//!
//! On Linux and the BSDs, the Documents, Downloads, Pictures, and similar
//! folders are not fixed: desktop environments localise them (for
//! example `~/Dokumente`) and users can move them. The authoritative
//! location is `$XDG_CONFIG_HOME/user-dirs.dirs`, a shell-style file of
//! lines such as:
//!
//! ```text
//! XDG_DOCUMENTS_DIR="$HOME/Documents"
//! XDG_DOWNLOAD_DIR="/data/downloads"
//! ```
//!
//! Per the xdg-user-dirs specification, a value is either `$HOME/...` or
//! an absolute path. Anything else is ignored. A value that resolves to
//! the home directory itself means the folder is disabled; callers then
//! fall back to the conventional name.
//!
//! The file is user-controlled input, so the parser is strict and
//! bounded: it reads at most [`MAX_FILE_BYTES`], accepts only the six
//! keys this crate uses, and never expands anything except a leading
//! `$HOME`.

use std::path::{Path, PathBuf};

use crate::join::clean;

/// Largest `user-dirs.dirs` file the parser will read.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub(crate) const MAX_FILE_BYTES: u64 = 64 * 1024;

/// Folders found in a `user-dirs.dirs` file. `None` means "not set".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct UserDirs {
    pub documents: Option<PathBuf>,
    pub desktop: Option<PathBuf>,
    pub download: Option<PathBuf>,
    pub pictures: Option<PathBuf>,
    pub music: Option<PathBuf>,
    pub videos: Option<PathBuf>,
}

/// Parses the contents of a `user-dirs.dirs` file.
pub(crate) fn parse(contents: &str, home: &Path) -> UserDirs {
    let mut dirs = UserDirs::default();
    for line in contents.lines() {
        let Some((key, value)) = split_assignment(line) else {
            continue;
        };
        let slot = match key {
            "XDG_DOCUMENTS_DIR" => &mut dirs.documents,
            "XDG_DESKTOP_DIR" => &mut dirs.desktop,
            "XDG_DOWNLOAD_DIR" => &mut dirs.download,
            "XDG_PICTURES_DIR" => &mut dirs.pictures,
            "XDG_MUSIC_DIR" => &mut dirs.music,
            "XDG_VIDEOS_DIR" => &mut dirs.videos,
            _ => continue,
        };
        if let Some(path) = resolve_value(value, home) {
            *slot = Some(path);
        }
    }
    dirs
}

/// Splits `KEY="value"` into its parts. Comments, blank lines, and
/// anything not double-quoted are rejected.
fn split_assignment(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, rest) = line.split_once('=')?;
    // The reference lookup in xdg-user-dirs skips whitespace around `=`,
    // so this parser does too.
    let rest = rest.trim();
    let value = rest.strip_prefix('"')?.strip_suffix('"')?;
    Some((key.trim(), value))
}

/// Resolves one value. Only `$HOME`, `$HOME/...`, and absolute paths are
/// accepted; the result must not be the home directory itself.
fn resolve_value(value: &str, home: &Path) -> Option<PathBuf> {
    if value.contains(['\0', '\n', '"', '`']) {
        return None;
    }
    let path = if let Some(rest) = value.strip_prefix("$HOME") {
        if !(rest.is_empty() || rest.starts_with('/')) {
            return None;
        }
        let rest = rest.trim_start_matches('/');
        if rest.contains('$') {
            return None;
        }
        clean(home.join(rest))
    } else if value.starts_with('/') && !value.contains('$') {
        clean(value)
    } else {
        return None;
    };
    if path == clean(home) {
        return None;
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> PathBuf {
        PathBuf::from("/home/u")
    }

    /// Expected value for `$HOME/<rel>`, cleaned the same way the parser
    /// cleans (so the test holds on Windows hosts too).
    fn h(rel: &str) -> PathBuf {
        clean(home().join(rel))
    }

    #[test]
    fn test_parse_standard_file() {
        let file = r#"
# This file is written by xdg-user-dirs-update
XDG_DESKTOP_DIR="$HOME/Desktop"
XDG_DOWNLOAD_DIR="$HOME/Downloads"
XDG_DOCUMENTS_DIR="$HOME/Dokumente"
XDG_MUSIC_DIR="/mnt/media/music"
XDG_PICTURES_DIR="$HOME/Pictures"
XDG_VIDEOS_DIR="$HOME/Videos"
XDG_TEMPLATES_DIR="$HOME/Templates"
"#;
        let d = parse(file, &home());
        assert_eq!(d.documents, Some(h("Dokumente")));
        assert_eq!(d.desktop, Some(h("Desktop")));
        assert_eq!(d.download, Some(h("Downloads")));
        assert_eq!(d.music, Some(clean("/mnt/media/music")));
        assert_eq!(d.pictures, Some(h("Pictures")));
        assert_eq!(d.videos, Some(h("Videos")));
    }

    #[test]
    fn test_parse_disabled_folder_pointing_at_home_is_ignored() {
        let d = parse(
            "XDG_DOCUMENTS_DIR=\"$HOME/\"\nXDG_DESKTOP_DIR=\"$HOME\"",
            &home(),
        );
        assert_eq!(d.documents, None);
        assert_eq!(d.desktop, None);
    }

    #[test]
    fn test_parse_rejects_relative_unquoted_and_expansions() {
        let file = "XDG_DOCUMENTS_DIR=\"Documents\"\n\
                    XDG_DESKTOP_DIR=$HOME/Desktop\n\
                    XDG_DOWNLOAD_DIR=\"$HOME/$(rm -rf ~)\"\n\
                    XDG_MUSIC_DIR=\"$HOMEBREW/music\"\n\
                    XDG_PICTURES_DIR=\"/a/$USER/b\"\n\
                    XDG_VIDEOS_DIR=\"`id`\"";
        assert_eq!(parse(file, &home()), UserDirs::default());
    }

    #[test]
    fn test_parse_cleans_traversal_in_values() {
        let d = parse("XDG_DOCUMENTS_DIR=\"$HOME/a/../docs//\"", &home());
        assert_eq!(d.documents, Some(h("docs")));
    }

    #[test]
    fn test_parse_tolerates_whitespace_and_comments() {
        let d = parse(
            "   # comment\n  XDG_DOCUMENTS_DIR = \"$HOME/D\"   \n\n",
            &home(),
        );
        assert_eq!(d.documents, Some(h("D")));
    }

    #[test]
    fn test_parse_later_valid_entry_wins() {
        let d = parse(
            "XDG_DOCUMENTS_DIR=\"$HOME/A\"\nXDG_DOCUMENTS_DIR=\"$HOME/B\"",
            &home(),
        );
        assert_eq!(d.documents, Some(h("B")));
    }

    #[test]
    fn test_parse_garbage_input_yields_nothing() {
        let d = parse("\0\0=\"\"\n=\n\"\"\nXDG_DOCUMENTS_DIR=\"\0\"", &home());
        assert_eq!(d, UserDirs::default());
    }
}
