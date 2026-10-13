//! Windows: Known Folders, with environment-variable fallbacks.
//!
//! | Kind      | User                                  | Global                          |
//! |-----------|---------------------------------------|---------------------------------|
//! | data      | `%LOCALAPPDATA%\<app>`                | `%ProgramData%\<app>`           |
//! | config    | `%APPDATA%\<app>` (roaming)           | `%ProgramData%\<app>\config`    |
//! | cache     | `%LOCALAPPDATA%\<app>\cache`          | `%ProgramData%\<app>\cache`     |
//! | logs      | `%LOCALAPPDATA%\<app>\logs`           | `%ProgramData%\<app>\logs`      |
//! | state     | `%LOCALAPPDATA%\<app>\state`          | `%ProgramData%\<app>\state`     |
//! | runtime   | `%LOCALAPPDATA%\<app>\run`            | `%ProgramData%\<app>\run`       |
//! | temp      | `%TEMP%\<app>`                        | `%ProgramData%\<app>\temp`      |
//! | documents | `<Documents>\<app>`                   | `%PUBLIC%\Documents\<app>`      |
//!
//! Folders are read with `SHGetKnownFolderPath`, which is the only
//! correct source on Windows: the Documents, Desktop, Pictures, and
//! other user folders are frequently redirected (`OneDrive` backup does
//! this by default on Windows 11), and no environment variable reflects
//! the redirection. When the shell call fails, the documented
//! environment variable is used; when that is missing too, the folder is
//! derived from the profile directory.
//!
//! Configuration goes to the roaming profile so it follows the user
//! across machines on a domain; data, caches, and logs stay local
//! because they can be large and are machine-specific.

use std::path::{Path, PathBuf};

use super::{Env, Folders, Layout, absolute_var};
use crate::join::clean;

/// Per-user bases on Windows.
#[derive(Debug, Clone)]
pub(crate) struct UserBases {
    pub folders: Folders,
    local: PathBuf,
    roaming: PathBuf,
    temp: PathBuf,
}

impl UserBases {
    pub(crate) fn resolve(env: Env<'_>) -> Result<Self, &'static str> {
        Self::resolve_with(env, &ffi::known_folder)
    }

    /// Resolution with an injectable Known Folder lookup, for tests.
    pub(crate) fn resolve_with(
        env: Env<'_>,
        known: &dyn Fn(KnownFolder) -> Option<PathBuf>,
    ) -> Result<Self, &'static str> {
        let home = known(KnownFolder::Profile)
            .or_else(|| absolute_var(env, "USERPROFILE"))
            .ok_or("home")?;
        let local = known(KnownFolder::LocalAppData)
            .or_else(|| absolute_var(env, "LOCALAPPDATA"))
            .unwrap_or_else(|| home.join("AppData").join("Local"));
        let roaming = known(KnownFolder::RoamingAppData)
            .or_else(|| absolute_var(env, "APPDATA"))
            .unwrap_or_else(|| home.join("AppData").join("Roaming"));
        let temp = absolute_var(env, "TMP")
            .or_else(|| absolute_var(env, "TEMP"))
            .unwrap_or_else(|| local.join("Temp"));
        let folder =
            |id: KnownFolder, fallback: &str| known(id).unwrap_or_else(|| home.join(fallback));
        let folders = Folders {
            documents: folder(KnownFolder::Documents, "Documents"),
            desktop: folder(KnownFolder::Desktop, "Desktop"),
            downloads: folder(KnownFolder::Downloads, "Downloads"),
            pictures: folder(KnownFolder::Pictures, "Pictures"),
            music: folder(KnownFolder::Music, "Music"),
            videos: folder(KnownFolder::Videos, "Videos"),
            home,
        };
        Ok(Self {
            folders,
            local,
            roaming,
            temp,
        })
    }

    pub(crate) fn layout(&self, name: &str) -> Layout {
        let data = self.local.join(name);
        Layout {
            config: self.roaming.join(name),
            cache: data.join("cache"),
            logs: data.join("logs"),
            state: data.join("state"),
            runtime: data.join("run"),
            temp: self.temp.join(name),
            documents: self.folders.documents.join(name),
            data,
        }
    }
}

/// Machine-wide bases on Windows.
#[derive(Debug, Clone)]
pub(crate) struct SystemBases {
    pub root: PathBuf,
    program_data: PathBuf,
    public_documents: PathBuf,
}

impl SystemBases {
    pub(crate) fn resolve(env: Env<'_>) -> Result<Self, &'static str> {
        Self::resolve_with(env, &ffi::known_folder)
    }

    pub(crate) fn resolve_with(
        env: Env<'_>,
        known: &dyn Fn(KnownFolder) -> Option<PathBuf>,
    ) -> Result<Self, &'static str> {
        let program_data = known(KnownFolder::ProgramData)
            .or_else(|| absolute_var(env, "ProgramData"))
            .ok_or("program data")?;
        let root = system_root(env, &program_data).ok_or("system root")?;
        let public_documents = known(KnownFolder::PublicDocuments)
            .or_else(|| absolute_var(env, "PUBLIC").map(|p| p.join("Documents")))
            .unwrap_or_else(|| root.join("Users").join("Public").join("Documents"));
        Ok(Self {
            root,
            program_data,
            public_documents,
        })
    }

    pub(crate) fn layout(&self, name: &str) -> Layout {
        let data = self.program_data.join(name);
        Layout {
            config: data.join("config"),
            cache: data.join("cache"),
            logs: data.join("logs"),
            state: data.join("state"),
            runtime: data.join("run"),
            temp: data.join("temp"),
            documents: self.public_documents.join(name),
            data,
        }
    }
}

/// The root of the system volume, such as `C:\`.
///
/// `%SystemDrive%` is authoritative; when it is missing or malformed the
/// drive of `%ProgramData%` is used, which is always on the system volume.
fn system_root(env: Env<'_>, program_data: &Path) -> Option<PathBuf> {
    if let Some(drive) = env("SystemDrive") {
        let b = drive.as_encoded_bytes();
        if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            let mut root = PathBuf::from(drive);
            root.push("\\");
            return Some(root);
        }
    }
    let mut components = program_data.components();
    let prefix = components.next()?;
    let root_dir = components.next()?;
    let mut root = PathBuf::new();
    root.push(prefix);
    root.push(root_dir);
    Some(clean(root))
}

/// The Known Folders this crate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KnownFolder {
    Profile,
    LocalAppData,
    RoamingAppData,
    ProgramData,
    PublicDocuments,
    Documents,
    Desktop,
    Downloads,
    Pictures,
    Music,
    Videos,
}

/// The `SHGetKnownFolderPath` binding.
///
/// Declared by hand instead of through `windows-sys` because the crate
/// needs exactly two functions and one struct; a dependency would add
/// tens of megabytes of bindings to the build for them.
#[allow(unsafe_code)]
mod ffi {
    use std::ffi::{OsString, c_void};
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    use super::KnownFolder;
    use crate::join::clean;

    /// A Win32 `GUID`. Layout matches `guiddef.h`.
    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    /// Return the path even if the folder does not exist yet.
    const KF_FLAG_DONT_VERIFY: u32 = 0x0000_4000;

    /// Longest path Windows can represent (`UNICODE_STRING` limit), in
    /// UTF-16 units. Bounds the terminator scan below.
    const MAX_WIDE_LEN: usize = 32_767;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            rfid: *const Guid,
            dw_flags: u32,
            h_token: *mut c_void,
            ppsz_path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(pv: *const c_void);
    }

    impl KnownFolder {
        /// `FOLDERID_*` values from `KnownFolders.h`.
        const fn guid(self) -> Guid {
            let (data1, data2, data3, data4) = match self {
                Self::Profile => (
                    0x5E6C_858F,
                    0x0E22,
                    0x4760,
                    [0x9A, 0xFE, 0xEA, 0x33, 0x17, 0xB6, 0x71, 0x73],
                ),
                Self::LocalAppData => (
                    0xF1B3_2785,
                    0x6FBA,
                    0x4FCF,
                    [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91],
                ),
                Self::RoamingAppData => (
                    0x3EB6_85DB,
                    0x65F9,
                    0x4CF6,
                    [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D],
                ),
                Self::ProgramData => (
                    0x62AB_5D82,
                    0xFDC1,
                    0x4DC3,
                    [0xA9, 0xDD, 0x07, 0x0D, 0x1D, 0x49, 0x5D, 0x97],
                ),
                Self::PublicDocuments => (
                    0xED48_24AF,
                    0xDCE4,
                    0x45A8,
                    [0x81, 0xE2, 0xFC, 0x79, 0x65, 0x08, 0x36, 0x34],
                ),
                Self::Documents => (
                    0xFDD3_9AD0,
                    0x238F,
                    0x46AF,
                    [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7],
                ),
                Self::Desktop => (
                    0xB4BF_CC3A,
                    0xDB2C,
                    0x424C,
                    [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41],
                ),
                Self::Downloads => (
                    0x374D_E290,
                    0x123F,
                    0x4565,
                    [0x91, 0x64, 0x39, 0xC4, 0x92, 0x5E, 0x46, 0x7B],
                ),
                Self::Pictures => (
                    0x33E2_8130,
                    0x4E1E,
                    0x4676,
                    [0x83, 0x5A, 0x98, 0x39, 0x5C, 0x3B, 0xC3, 0xBB],
                ),
                Self::Music => (
                    0x4BD8_D571,
                    0x6D19,
                    0x48D3,
                    [0xBE, 0x97, 0x42, 0x22, 0x20, 0x08, 0x0E, 0x43],
                ),
                Self::Videos => (
                    0x1898_9B1D,
                    0x99B5,
                    0x455B,
                    [0x84, 0x1C, 0xAB, 0x7C, 0x74, 0xE4, 0xDD, 0xFC],
                ),
            };
            Guid {
                data1,
                data2,
                data3,
                data4,
            }
        }
    }

    /// Looks up a Known Folder. `None` when the shell reports failure or
    /// returns something that is not an absolute path.
    pub(super) fn known_folder(folder: KnownFolder) -> Option<PathBuf> {
        let guid = folder.guid();
        let mut raw: *mut u16 = std::ptr::null_mut();
        // SAFETY: `guid` is a valid, live `GUID` for the duration of the
        // call; a null token selects the current user, as documented; and
        // `raw` is a valid out-pointer the function writes exactly once.
        let hr = unsafe {
            SHGetKnownFolderPath(
                &raw const guid,
                KF_FLAG_DONT_VERIFY,
                std::ptr::null_mut(),
                &raw mut raw,
            )
        };
        let path = if hr >= 0 && !raw.is_null() {
            // SAFETY: on success `raw` points to a NUL-terminated UTF-16
            // string allocated by the shell, which stays valid until the
            // `CoTaskMemFree` call below.
            unsafe { read_wide(raw) }
        } else {
            None
        };
        // SAFETY: the API contract requires the caller to free the output
        // with `CoTaskMemFree` whether or not the call succeeded, and
        // `CoTaskMemFree` accepts null. `raw` is not used after this.
        unsafe { CoTaskMemFree(raw.cast::<c_void>().cast_const()) };
        path.filter(|p| p.is_absolute()).map(clean)
    }

    /// Copies a NUL-terminated UTF-16 string into a `PathBuf`.
    ///
    /// Returns `None` when no terminator is found within
    /// [`MAX_WIDE_LEN`] units, which would indicate a corrupt buffer.
    ///
    /// # Safety
    ///
    /// `ptr` must be non-null and point to a readable buffer of `u16`
    /// that contains a NUL terminator, or that is at least
    /// [`MAX_WIDE_LEN`] units long.
    unsafe fn read_wide(ptr: *const u16) -> Option<PathBuf> {
        let mut len = 0usize;
        loop {
            if len >= MAX_WIDE_LEN {
                return None;
            }
            // SAFETY: `len < MAX_WIDE_LEN`, and the caller guarantees the
            // buffer is readable up to the terminator or that limit.
            if unsafe { *ptr.add(len) } == 0 {
                break;
            }
            len += 1;
        }
        // SAFETY: the `len` units before the terminator were just read
        // successfully, so they form a valid, initialised slice.
        let units = unsafe { std::slice::from_raw_parts(ptr, len) };
        Some(PathBuf::from(OsString::from_wide(units)))
    }

    #[cfg(test)]
    #[allow(clippy::unwrap_used)]
    mod tests {
        use super::*;

        #[test]
        fn test_known_folder_local_app_data_matches_environment() {
            let from_shell = known_folder(KnownFolder::LocalAppData).unwrap();
            let from_env = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
            assert_eq!(from_shell, clean(from_env));
        }

        #[test]
        fn test_known_folder_every_id_resolves_to_absolute_path() {
            for id in [
                KnownFolder::Profile,
                KnownFolder::LocalAppData,
                KnownFolder::RoamingAppData,
                KnownFolder::ProgramData,
                KnownFolder::PublicDocuments,
                KnownFolder::Documents,
                KnownFolder::Desktop,
                KnownFolder::Downloads,
                KnownFolder::Pictures,
                KnownFolder::Music,
                KnownFolder::Videos,
            ] {
                let p = known_folder(id);
                assert!(
                    p.as_ref().is_some_and(|p| p.is_absolute()),
                    "{id:?} -> {p:?}"
                );
            }
        }

        #[test]
        fn test_read_wide_stops_at_terminator() {
            let buf: Vec<u16> = "C:\\x\0garbage".encode_utf16().collect();
            // SAFETY: `buf` is a live, NUL-terminated buffer.
            let p = unsafe { read_wide(buf.as_ptr()) };
            assert_eq!(p, Some(PathBuf::from("C:\\x")));
        }

        #[test]
        fn test_read_wide_rejects_unterminated_buffer() {
            let buf = vec![u16::from(b'a'); MAX_WIDE_LEN];
            // SAFETY: `buf` is exactly `MAX_WIDE_LEN` units long.
            assert_eq!(unsafe { read_wide(buf.as_ptr()) }, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_env::{assert_distinct, env_from};
    use super::*;

    fn none(_: KnownFolder) -> Option<PathBuf> {
        None
    }

    #[test]
    fn test_user_uses_known_folders_first() {
        let known = |id: KnownFolder| match id {
            KnownFolder::Profile => Some(PathBuf::from(r"C:\Users\u")),
            KnownFolder::Documents => Some(PathBuf::from(r"D:\OneDrive\Documents")),
            KnownFolder::LocalAppData => Some(PathBuf::from(r"C:\Users\u\AppData\Local")),
            _ => None,
        };
        let env = env_from(&[("TEMP", r"C:\Users\u\AppData\Local\Temp")]);
        let l = UserBases::resolve_with(&env, &known)
            .map(|b| b.layout("HiveDB"))
            .ok();
        let l = l.as_ref();
        assert_eq!(
            l.map(|l| l.documents.as_path()),
            Some(Path::new(r"D:\OneDrive\Documents\HiveDB"))
        );
        assert_eq!(
            l.map(|l| l.data.as_path()),
            Some(Path::new(r"C:\Users\u\AppData\Local\HiveDB"))
        );
        assert_eq!(
            l.map(|l| l.config.as_path()),
            Some(Path::new(r"C:\Users\u\AppData\Roaming\HiveDB"))
        );
        assert_eq!(
            l.map(|l| l.temp.as_path()),
            Some(Path::new(r"C:\Users\u\AppData\Local\Temp\HiveDB"))
        );
        if let Some(l) = l {
            assert_distinct(l);
        }
    }

    #[test]
    fn test_user_falls_back_to_environment() {
        let env = env_from(&[
            ("USERPROFILE", r"C:\Users\u"),
            ("LOCALAPPDATA", "C:/Users/u/AppData/Local/"),
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
        ]);
        let b = UserBases::resolve_with(&env, &none).ok();
        assert_eq!(
            b.as_ref().map(|b| b.layout("a").data),
            Some(PathBuf::from(r"C:\Users\u\AppData\Local\a"))
        );
        assert_eq!(
            b.map(|b| b.folders.documents),
            Some(PathBuf::from(r"C:\Users\u\Documents"))
        );
    }

    #[test]
    fn test_user_without_home_fails() {
        let env = env_from(&[("USERPROFILE", "relative")]);
        assert_eq!(UserBases::resolve_with(&env, &none).err(), Some("home"));
    }

    #[test]
    fn test_system_layout_and_root() {
        let env = env_from(&[
            ("ProgramData", r"C:\ProgramData"),
            ("SystemDrive", "C:"),
            ("PUBLIC", r"C:\Users\Public"),
        ]);
        let s = SystemBases::resolve_with(&env, &none).ok();
        assert_eq!(
            s.as_ref().map(|s| s.root.clone()),
            Some(PathBuf::from(r"C:\"))
        );
        let l = s.map(|s| s.layout("HiveDB"));
        let l = l.as_ref();
        assert_eq!(
            l.map(|l| l.data.as_path()),
            Some(Path::new(r"C:\ProgramData\HiveDB"))
        );
        assert_eq!(
            l.map(|l| l.documents.as_path()),
            Some(Path::new(r"C:\Users\Public\Documents\HiveDB"))
        );
        if let Some(l) = l {
            assert_distinct(l);
        }
    }

    #[test]
    fn test_system_root_derived_from_program_data_when_drive_missing_or_bad() {
        for drive in [None, Some("C"), Some("C:\\x"), Some("1:")] {
            let mut pairs = vec![("ProgramData", r"E:\ProgramData")];
            if let Some(d) = drive {
                pairs.push(("SystemDrive", d));
            }
            let env = env_from(&pairs);
            let s = SystemBases::resolve_with(&env, &none).ok();
            assert_eq!(s.map(|s| s.root), Some(PathBuf::from(r"E:\")), "{drive:?}");
        }
    }

    #[test]
    fn test_system_without_program_data_fails() {
        assert_eq!(
            SystemBases::resolve_with(&env_from(&[]), &none).err(),
            Some("program data")
        );
    }
}
