//! Character and segment rules shared by every validator in the crate.
//!
//! The rules are the intersection of what Linux, macOS, and Windows
//! accept, plus a short list of Unicode characters that are legal on all
//! three but dangerous: look-alike separators (which some Windows code
//! pages "best-fit" back to `/` or `\`) and bidirectional overrides
//! (which make a name render differently from its bytes). A segment that
//! passes [`check_segment`] is a valid, unambiguous name on every
//! supported platform.
//!
//! Validation runs before any path is built: inputs are rejected, never
//! silently repaired. [`crate::sanitize`] exists for callers who want a
//! lossy repair instead.

use crate::error::Reason;

/// Longest accepted single segment, in UTF-8 bytes.
///
/// 255 is the per-name limit on ext4, APFS, and NTFS. Counting bytes is
/// the strictest of the three (NTFS counts UTF-16 units, which is never
/// more than the UTF-8 byte count), so a segment within this limit fits
/// everywhere.
pub const MAX_SEGMENT_BYTES: usize = 255;

/// Longest accepted relative sub-path, in UTF-8 bytes.
///
/// Matches Linux `PATH_MAX`. Bounding the input keeps validation cost
/// bounded no matter what a caller passes in.
pub const MAX_INPUT_BYTES: usize = 4096;

/// Most segments accepted in one relative sub-path.
pub const MAX_DEPTH: usize = 128;

/// Characters reserved by Windows (and `/`, `\` everywhere).
#[inline]
pub(crate) const fn is_reserved_ascii(b: u8) -> bool {
    matches!(b, b'<' | b'>' | b':' | b'"' | b'|' | b'?' | b'*')
}

/// Non-ASCII characters that look like, or best-fit map to, a reserved
/// ASCII character.
#[inline]
pub(crate) const fn is_lookalike(c: char) -> bool {
    matches!(
        c,
        '\u{2044}' // FRACTION SLASH
            | '\u{2215}' // DIVISION SLASH
            | '\u{2216}' // SET MINUS
            | '\u{29F8}' // BIG SOLIDUS
            | '\u{29F9}' // BIG REVERSE SOLIDUS
            | '\u{FE68}' // SMALL REVERSE SOLIDUS
            | '\u{FF02}' // FULLWIDTH QUOTATION MARK
            | '\u{FF0A}' // FULLWIDTH ASTERISK
            | '\u{FF0F}' // FULLWIDTH SOLIDUS
            | '\u{FF1A}' // FULLWIDTH COLON
            | '\u{FF1C}' // FULLWIDTH LESS-THAN SIGN
            | '\u{FF1E}' // FULLWIDTH GREATER-THAN SIGN
            | '\u{FF1F}' // FULLWIDTH QUESTION MARK
            | '\u{FF3C}' // FULLWIDTH REVERSE SOLIDUS
            | '\u{FF5C}' // FULLWIDTH VERTICAL LINE
    )
}

/// Bidirectional embeddings, overrides, isolates, and the byte-order
/// mark. Joiners (U+200C, U+200D) and directional marks (U+200E, U+200F)
/// are allowed because real scripts depend on them.
#[inline]
pub(crate) const fn is_invisible(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}')
}

/// Whether a single segment is a Windows device name (`CON`, `NUL`,
/// `COM1`, `LPT¹`, `CONIN$`, ...), with or without an extension.
///
/// Windows matches the part before the first `.` with trailing spaces
/// removed, case-insensitively.
pub(crate) fn is_reserved_name(segment: &str) -> bool {
    let stem = match segment.find('.') {
        Some(dot) => &segment[..dot],
        None => segment,
    };
    let stem = stem.trim_end_matches(' ');
    let bytes = stem.as_bytes();
    match bytes.len() {
        3 => {
            let up = [
                bytes[0].to_ascii_uppercase(),
                bytes[1].to_ascii_uppercase(),
                bytes[2].to_ascii_uppercase(),
            ];
            matches!(&up, b"CON" | b"PRN" | b"AUX" | b"NUL")
        }
        4 | 5 => {
            let head = [
                bytes[0].to_ascii_uppercase(),
                bytes[1].to_ascii_uppercase(),
                bytes[2].to_ascii_uppercase(),
            ];
            if !matches!(&head, b"COM" | b"LPT") {
                return false;
            }
            matches!(
                &stem[3..],
                "0" | "1"
                    | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "7"
                    | "8"
                    | "9"
                    | "\u{00B9}"
                    | "\u{00B2}"
                    | "\u{00B3}"
            )
        }
        6 | 7 => stem.eq_ignore_ascii_case("CONIN$") || stem.eq_ignore_ascii_case("CONOUT$"),
        _ => false,
    }
}

/// Validates one path segment.
///
/// `allow_wildcards` lets `*` and `?` through for search patterns; every
/// other rule still applies. `.` and `..` are not special here: callers
/// that give them meaning (the sub-path joiner) handle them first, and
/// everywhere else they fail the trailing-dot rule.
pub(crate) fn check_segment(segment: &str, allow_wildcards: bool) -> Result<(), Reason> {
    if segment.is_empty() {
        return Err(Reason::Empty);
    }
    if segment.len() > MAX_SEGMENT_BYTES {
        return Err(Reason::SegmentTooLong);
    }
    if segment.is_ascii() {
        for &b in segment.as_bytes() {
            check_ascii(b, allow_wildcards)?;
        }
    } else {
        for c in segment.chars() {
            if c.is_ascii() {
                // `c` is ASCII, so the cast is lossless.
                check_ascii(c as u8, allow_wildcards)?;
            } else if c.is_control() {
                return Err(Reason::ControlChar);
            } else if is_lookalike(c) {
                return Err(Reason::ReservedChar(c));
            } else if is_invisible(c) {
                return Err(Reason::InvisibleChar(c));
            }
        }
    }
    if segment.ends_with(['.', ' ']) {
        return Err(Reason::TrailingDotOrSpace);
    }
    if is_reserved_name(segment) {
        return Err(Reason::ReservedName);
    }
    Ok(())
}

#[inline]
fn check_ascii(b: u8, allow_wildcards: bool) -> Result<(), Reason> {
    if b < 0x20 || b == 0x7F {
        Err(Reason::ControlChar)
    } else if b == b'/' || b == b'\\' {
        Err(Reason::Separator)
    } else if is_reserved_ascii(b) && !(allow_wildcards && (b == b'*' || b == b'?')) {
        Err(Reason::ReservedChar(char::from(b)))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_segment_plain_names_pass() {
        for ok in [
            "a", "data", "my file", "v1.2.3", "résumé", "名前", "a-b_c", "~x", ".hidden",
        ] {
            assert_eq!(check_segment(ok, false), Ok(()), "{ok:?}");
        }
    }

    #[test]
    fn test_check_segment_empty_returns_empty() {
        assert_eq!(check_segment("", false), Err(Reason::Empty));
    }

    #[test]
    fn test_check_segment_at_length_limit_passes_and_over_fails() {
        let at = "a".repeat(MAX_SEGMENT_BYTES);
        let over = "a".repeat(MAX_SEGMENT_BYTES + 1);
        assert_eq!(check_segment(&at, false), Ok(()));
        assert_eq!(check_segment(&over, false), Err(Reason::SegmentTooLong));
    }

    #[test]
    fn test_check_segment_multibyte_length_counts_bytes() {
        // 128 two-byte chars = 256 bytes.
        let s = "é".repeat(128);
        assert_eq!(check_segment(&s, false), Err(Reason::SegmentTooLong));
    }

    #[test]
    fn test_check_segment_reserved_ascii_rejected() {
        for c in ['<', '>', ':', '"', '|', '?', '*'] {
            let s = format!("a{c}b");
            assert_eq!(
                check_segment(&s, false),
                Err(Reason::ReservedChar(c)),
                "{s:?}"
            );
        }
    }

    #[test]
    fn test_check_segment_wildcards_allowed_only_when_requested() {
        assert_eq!(check_segment("*.log", true), Ok(()));
        assert_eq!(check_segment("a?c", true), Ok(()));
        assert_eq!(check_segment("a:c", true), Err(Reason::ReservedChar(':')));
    }

    #[test]
    fn test_check_segment_separators_rejected() {
        assert_eq!(check_segment("a/b", false), Err(Reason::Separator));
        assert_eq!(check_segment("a\\b", false), Err(Reason::Separator));
    }

    #[test]
    fn test_check_segment_control_chars_rejected() {
        for s in ["a\0b", "a\nb", "a\tb", "a\u{7f}b", "a\u{85}b", "a\u{9b}b"] {
            assert_eq!(check_segment(s, false), Err(Reason::ControlChar), "{s:?}");
        }
    }

    #[test]
    fn test_check_segment_lookalikes_rejected() {
        for c in ['\u{2215}', '\u{FF0F}', '\u{FF3C}', '\u{FF1A}', '\u{2044}'] {
            let s = format!("a{c}b");
            assert_eq!(check_segment(&s, false), Err(Reason::ReservedChar(c)));
        }
    }

    #[test]
    fn test_check_segment_bidi_override_rejected_but_joiners_allowed() {
        assert_eq!(
            check_segment("evil\u{202E}txt.exe", false),
            Err(Reason::InvisibleChar('\u{202E}'))
        );
        assert_eq!(
            check_segment("\u{FEFF}bom", false),
            Err(Reason::InvisibleChar('\u{FEFF}'))
        );
        assert_eq!(check_segment("می\u{200C}خواهم", false), Ok(()));
    }

    #[test]
    fn test_check_segment_trailing_dot_or_space_rejected() {
        for s in ["a.", "a ", ".", "..", "a. ", " "] {
            assert_eq!(
                check_segment(s, false),
                Err(Reason::TrailingDotOrSpace),
                "{s:?}"
            );
        }
    }

    #[test]
    fn test_reserved_names_detected_case_and_extension_insensitive() {
        for s in [
            "CON",
            "con",
            "Con.txt",
            "NUL",
            "nul.tar.gz",
            "AUX",
            "PRN",
            "COM1",
            "com9",
            "LPT0",
            "lpt5.log",
            "COM\u{00B9}",
            "LPT\u{00B3}.x",
            "CONIN$",
            "conout$",
            "NUL .txt",
        ] {
            assert!(is_reserved_name(s), "{s:?}");
        }
    }

    #[test]
    fn test_reserved_names_do_not_match_lookalikes() {
        for s in [
            "CONSOLE",
            "COM10",
            "COMA",
            "LPT",
            "nul_",
            "xcon",
            "AUXI.txt",
            "COM\u{00B4}",
        ] {
            assert!(!is_reserved_name(s), "{s:?}");
        }
    }
}
