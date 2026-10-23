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

/// Creates one directory private to the current user (see
/// [`App::ensure_private`](crate::App::ensure_private)).
pub(crate) fn create_private_dir(path: &Path) -> std::io::Result<()> {
    private_dir::create(path)
}

/// The DACL of `path` in SDDL form, for tests.
#[cfg(test)]
pub(crate) fn private_dir_dacl(path: &Path) -> String {
    private_dir::tests::dacl_of(path)
}

/// The binding behind [`create_private_dir`]: the current user's SID, an
/// SDDL security descriptor, and `CreateDirectoryW`.
///
/// Declared by hand for the same reason as [`ffi`]: six functions and two
/// structs do not justify a bindings dependency.
#[allow(unsafe_code)]
mod private_dir {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    /// `SECURITY_ATTRIBUTES` from `minwinbase.h`.
    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        security_descriptor: *mut c_void,
        inherit_handle: i32,
    }

    /// `SID_AND_ATTRIBUTES` from `winnt.h`, the start of `TOKEN_USER`.
    #[repr(C)]
    struct SidAndAttributes {
        sid: *mut c_void,
        attributes: u32,
    }

    const TOKEN_QUERY: u32 = 0x0008;
    /// `TokenUser` in `TOKEN_INFORMATION_CLASS`.
    const TOKEN_USER: i32 = 1;
    const SDDL_REVISION_1: u32 = 1;
    /// Paths longer than this use the `\\?\` form, which `CreateDirectoryW`
    /// needs past `MAX_PATH` (260) minus room for an 8.3 file name.
    const LONG_PATH: usize = 248;
    /// Upper bound for a SID's string form; real ones are under 200 units.
    const MAX_SID_LEN: usize = 256;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
        fn CreateDirectoryW(path: *const u16, attributes: *const SecurityAttributes) -> i32;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
        fn GetTokenInformation(
            token: *mut c_void,
            class: i32,
            information: *mut c_void,
            length: u32,
            returned: *mut u32,
        ) -> i32;
        fn ConvertSidToStringSidW(sid: *mut c_void, string: *mut *mut u16) -> i32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
    }

    /// Creates `path` (one level; its parent must exist) with a protected
    /// DACL: full control for the current user, `SYSTEM`, and
    /// `Administrators`, inherited by everything created inside.
    pub(super) fn create(path: &Path) -> io::Result<()> {
        let sddl = descriptor_for(&current_user_sid()?);
        let sddl: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        // SAFETY: `sddl` is a live, NUL-terminated UTF-16 string;
        // `descriptor` is a valid out-pointer, and a null size pointer is
        // allowed by the API.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || descriptor.is_null() {
            return Err(io::Error::last_os_error());
        }
        let attributes = SecurityAttributes {
            length: u32::try_from(size_of::<SecurityAttributes>()).unwrap_or(u32::MAX),
            security_descriptor: descriptor,
            inherit_handle: 0,
        };
        let wide = wide_path(path);
        // SAFETY: `wide` is a live, NUL-terminated UTF-16 path, and
        // `attributes` points to a valid descriptor that stays allocated
        // until the `LocalFree` below.
        let created = unsafe { CreateDirectoryW(wide.as_ptr(), &raw const attributes) };
        let result = if created == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        };
        // SAFETY: `descriptor` was allocated by the conversion above with
        // `LocalAlloc`, as documented, and is not used after this.
        // LocalFree returns null on success; there is nothing to do if it fails.
        let _freed = unsafe { LocalFree(descriptor) };
        result
    }

    /// The SDDL for a private directory owned by `user`. `P` blocks
    /// inheritance from the parent; `OICI` passes the entries on to files
    /// and directories created inside; `FA` is full access.
    pub(super) fn descriptor_for(user: &str) -> String {
        format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
    }

    /// The current user's SID in string form (`S-1-5-21-...`).
    pub(super) fn current_user_sid() -> io::Result<String> {
        let mut token: *mut c_void = std::ptr::null_mut();
        // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no
        // closing; `token` is a valid out-pointer.
        let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) };
        if opened == 0 {
            return Err(io::Error::last_os_error());
        }
        let sid = user_sid_string(token);
        // SAFETY: `token` was opened above and is closed exactly once.
        // A failure to close leaks one handle; the SID is still valid.
        let _closed = unsafe { CloseHandle(token) };
        sid
    }

    fn user_sid_string(token: *mut c_void) -> io::Result<String> {
        let mut needed = 0u32;
        // SAFETY: a null buffer with length 0 asks for the size, which the
        // call writes to `needed`; the call is expected to fail here.
        let _size_query = unsafe {
            GetTokenInformation(token, TOKEN_USER, std::ptr::null_mut(), 0, &raw mut needed)
        };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        // u64 storage keeps the pointer-sized fields aligned.
        let words = usize::try_from(needed).unwrap_or(usize::MAX).div_ceil(8);
        let mut buffer = vec![0u64; words];
        // SAFETY: `buffer` holds at least `needed` bytes and lives across
        // the call; `needed` is the size the system asked for.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TOKEN_USER,
                buffer.as_mut_ptr().cast::<c_void>(),
                needed,
                &raw mut needed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: on success the buffer starts with a `TOKEN_USER`, whose
        // first member is a `SID_AND_ATTRIBUTES`, and it is 8-byte aligned.
        let sid = unsafe { (*buffer.as_ptr().cast::<SidAndAttributes>()).sid };
        let mut string: *mut u16 = std::ptr::null_mut();
        // SAFETY: `sid` points into `buffer`, which is still alive; `string`
        // is a valid out-pointer.
        let converted = unsafe { ConvertSidToStringSidW(sid, &raw mut string) };
        if converted == 0 || string.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut units = Vec::with_capacity(64);
        for offset in 0..MAX_SID_LEN {
            // SAFETY: `string` is NUL-terminated (the scan stops there) and
            // a SID's string form is far shorter than `MAX_SID_LEN`.
            let unit = unsafe { *string.add(offset) };
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        // SAFETY: `string` was allocated by the system with `LocalAlloc` and
        // is not used after this.
        let _freed = unsafe { LocalFree(string.cast::<c_void>()) };
        String::from_utf16(&units).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// The path as a NUL-terminated wide string, in the `\\?\` form when it
    /// is long.
    fn wide_path(path: &Path) -> Vec<u16> {
        let units: Vec<u16> = path.as_os_str().encode_wide().collect();
        let mut wide = Vec::with_capacity(units.len() + 8);
        let backslash = u16::from(b'\\');
        let already_verbatim =
            units.starts_with(&[backslash, backslash, u16::from(b'?'), backslash]);
        if units.len() >= LONG_PATH && !already_verbatim {
            if units.starts_with(&[backslash, backslash]) {
                wide.extend(r"\\?\UNC".encode_utf16());
                wide.extend(
                    units
                        .iter()
                        .skip(1)
                        .map(|&u| if u == u16::from(b'/') { backslash } else { u }),
                );
            } else {
                wide.extend(r"\\?\".encode_utf16());
                wide.extend(
                    units
                        .iter()
                        .map(|&u| if u == u16::from(b'/') { backslash } else { u }),
                );
            }
        } else {
            wide.extend(units);
        }
        wide.push(0);
        wide
    }

    #[cfg(test)]
    #[allow(clippy::unwrap_used)]
    pub(crate) mod tests {
        use super::*;

        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn GetNamedSecurityInfoW(
                name: *const u16,
                object: u32,
                info: u32,
                owner: *mut *mut c_void,
                group: *mut *mut c_void,
                dacl: *mut *mut c_void,
                sacl: *mut *mut c_void,
                descriptor: *mut *mut c_void,
            ) -> u32;
            fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor: *mut c_void,
                revision: u32,
                info: u32,
                string: *mut *mut u16,
                length: *mut u32,
            ) -> i32;
        }

        const SE_FILE_OBJECT: u32 = 1;
        const DACL_SECURITY_INFORMATION: u32 = 4;

        /// The directory's DACL in SDDL form, read back from the system.
        pub(crate) fn dacl_of(path: &Path) -> String {
            let wide = wide_path(path);
            let mut descriptor: *mut c_void = std::ptr::null_mut();
            let mut dacl: *mut c_void = std::ptr::null_mut();
            // SAFETY: `wide` is NUL-terminated; every out-pointer is valid
            // or null where the API allows it.
            let status = unsafe {
                GetNamedSecurityInfoW(
                    wide.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &raw mut dacl,
                    std::ptr::null_mut(),
                    &raw mut descriptor,
                )
            };
            assert_eq!(status, 0, "GetNamedSecurityInfoW failed");
            let text = dacl_string(descriptor);
            // SAFETY: allocated by the system with `LocalAlloc`.
            let _freed = unsafe { LocalFree(descriptor) };
            text
        }

        /// The DACL part of `sddl` as the system writes it back: SIDs with
        /// a well-known short name (`LA` for the built-in Administrator,
        /// for example) come back as that name.
        pub(crate) fn canonical(sddl: &str) -> String {
            let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
            let mut descriptor: *mut c_void = std::ptr::null_mut();
            // SAFETY: as in `create`.
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    SDDL_REVISION_1,
                    &raw mut descriptor,
                    std::ptr::null_mut(),
                )
            };
            assert_ne!(ok, 0);
            let text = dacl_string(descriptor);
            // SAFETY: allocated by the conversion with `LocalAlloc`.
            let _freed = unsafe { LocalFree(descriptor) };
            text
        }

        fn dacl_string(descriptor: *mut c_void) -> String {
            let mut string: *mut u16 = std::ptr::null_mut();
            let mut length = 0u32;
            // SAFETY: `descriptor` is a valid security descriptor owned by
            // the caller; `string` and `length` are valid out-pointers.
            let ok = unsafe {
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor,
                    SDDL_REVISION_1,
                    DACL_SECURITY_INFORMATION,
                    &raw mut string,
                    &raw mut length,
                )
            };
            assert_ne!(ok, 0);
            // SAFETY: `string` holds `length` units, terminator included.
            let units = unsafe { std::slice::from_raw_parts(string, length as usize) };
            let text = String::from_utf16_lossy(units)
                .trim_end_matches('\0')
                .to_owned();
            // SAFETY: allocated by the system with `LocalAlloc`.
            let _freed = unsafe { LocalFree(string.cast::<c_void>()) };
            text
        }

        #[test]
        fn test_current_user_sid_is_a_sid() {
            let sid = current_user_sid().unwrap();
            assert!(sid.starts_with("S-1-"), "{sid}");
        }

        #[test]
        fn test_descriptor_for_is_protected_and_lists_three_trustees() {
            let sddl = descriptor_for("S-1-5-21-1-2-3-1001");
            assert!(sddl.starts_with("D:P"));
            assert_eq!(sddl.matches("(A;OICI;FA;;;").count(), 3);
        }

        #[test]
        fn test_create_sets_a_private_dacl() {
            let dir = std::env::temp_dir().join(format!("path-lib-private-{}", std::process::id()));
            drop(std::fs::remove_dir_all(&dir));
            create(&dir).unwrap();
            let dacl = dacl_of(&dir);
            let expected = canonical(&descriptor_for(&current_user_sid().unwrap()));
            // Exactly the protected list we set: the user, SYSTEM, and
            // Administrators, nothing inherited, nobody else.
            assert_eq!(dacl, expected);
            assert!(dacl.starts_with("D:P"), "not protected: {dacl}");
            assert_eq!(dacl.matches("(A;").count(), 3, "{dacl}");
            std::fs::remove_dir_all(&dir).unwrap();
        }

        #[test]
        fn test_create_twice_reports_already_exists() {
            let dir =
                std::env::temp_dir().join(format!("path-lib-private-twice-{}", std::process::id()));
            drop(std::fs::remove_dir_all(&dir));
            create(&dir).unwrap();
            let again = create(&dir).unwrap_err();
            assert_eq!(again.kind(), io::ErrorKind::AlreadyExists);
            std::fs::remove_dir_all(&dir).unwrap();
        }

        #[test]
        fn test_wide_path_uses_verbatim_form_only_when_long() {
            let short = wide_path(Path::new(r"C:\short"));
            assert_eq!(String::from_utf16_lossy(&short), "C:\\short\0");
            let long = format!(r"C:\{}", "a".repeat(300));
            let wide = String::from_utf16_lossy(&wide_path(Path::new(&long)));
            assert!(wide.starts_with(r"\\?\C:\"), "{wide}");
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
