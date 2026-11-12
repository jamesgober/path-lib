//! Who besides the trusted accounts can change or read an open file.
//!
//! Everything is read from the file's handle, never by path, so the file
//! checked is the file the caller reads. Two system calls do the work:
//! `GetVolumeInformationByHandleW` says whether the volume keeps access
//! control lists at all, and `GetSecurityInfo` returns the owner and the
//! DACL as one self-relative security descriptor. The descriptor is then
//! parsed as a byte slice, with every offset and length checked against
//! the slice, so the only `unsafe` is at the system boundary.
//!
//! SIDs are compared byte for byte, which is what `EqualSid` does for two
//! valid SIDs.

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;

use crate::FileAccess;

/// `SE_FILE_OBJECT` in `SE_OBJECT_TYPE`.
const SE_FILE_OBJECT: u32 = 1;
const OWNER_SECURITY_INFORMATION: u32 = 0x1;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
/// The volume stores and enforces access control lists.
const FILE_PERSISTENT_ACLS: u32 = 0x8;

/// `SECURITY_DESCRIPTOR_CONTROL` bits.
const SE_DACL_PRESENT: u16 = 0x0004;
const SE_SELF_RELATIVE: u16 = 0x8000;
/// Size of `SECURITY_DESCRIPTOR_RELATIVE`.
const DESCRIPTOR_HEADER: usize = 20;
/// Size of the `ACL` header.
const ACL_HEADER: usize = 8;
/// Size of `ACE_HEADER`.
const ACE_HEADER: usize = 4;
/// Offset of the access mask, and of the SID for the simple ACE types.
const ACE_MASK: usize = 4;
const ACE_SID: usize = 8;
/// A SID has at most 15 sub-authorities (`SID_MAX_SUB_AUTHORITIES`).
const MAX_SUB_AUTHORITIES: u8 = 15;

/// ACE types that grant access. The simple ones carry the trustee's SID
/// right after the mask; the others put flags, GUIDs, or a second SID
/// first, and never appear on files, so their trustee is not read: they
/// count as granting their rights to another account.
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0x0;
const ACCESS_ALLOWED_COMPOUND_ACE_TYPE: u8 = 0x4;
const ACCESS_ALLOWED_OBJECT_ACE_TYPE: u8 = 0x5;
const ACCESS_ALLOWED_CALLBACK_ACE_TYPE: u8 = 0x9;
const ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE: u8 = 0xB;
/// The entry only exists to be inherited; it grants nothing on this file.
const INHERIT_ONLY_ACE: u8 = 0x08;

/// Rights that let an account change the file, its attributes, or its
/// security: `FILE_WRITE_DATA`, `FILE_APPEND_DATA`, `FILE_WRITE_EA`,
/// `FILE_DELETE_CHILD` (for a directory handle), `FILE_WRITE_ATTRIBUTES`,
/// `DELETE`, `WRITE_DAC`, `WRITE_OWNER`, `MAXIMUM_ALLOWED`, `GENERIC_ALL`,
/// and `GENERIC_WRITE`.
const WRITE_RIGHTS: u32 = 0x0000_0002
    | 0x0000_0004
    | 0x0000_0010
    | 0x0000_0040
    | 0x0000_0100
    | 0x0001_0000
    | 0x0004_0000
    | 0x0008_0000
    | 0x0200_0000
    | 0x1000_0000
    | 0x4000_0000;
/// Rights that let an account read the contents: `FILE_READ_DATA`,
/// `MAXIMUM_ALLOWED`, `GENERIC_ALL`, and `GENERIC_READ`.
const READ_RIGHTS: u32 = 0x0000_0001 | 0x0200_0000 | 0x1000_0000 | 0x8000_0000;

/// `S-1-5-18`, `NT AUTHORITY\SYSTEM`. A SID is a revision byte, a
/// sub-authority count, a 48-bit big-endian authority, and that many
/// little-endian 32-bit sub-authorities.
const SYSTEM: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
/// `S-1-5-32-544`, `BUILTIN\Administrators`.
const ADMINISTRATORS: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x20, 0x02, 0, 0];
/// `S-1-3-4`, `OWNER RIGHTS`: an entry for whoever owns the file.
const OWNER_RIGHTS: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 3, 4, 0, 0, 0];
/// `S-1-3-0`, `CREATOR OWNER`: in an inheritable entry, replaced by the
/// account that creates the child.
const CREATOR_OWNER: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0];

/// The answer for a volume without access control lists: the fields
/// carry no information, so they hold the unsafe values and a check that
/// ignores `enforced` still refuses.
const UNENFORCED: FileAccess = FileAccess {
    enforced: false,
    owner_trusted: false,
    others_can_write: true,
    others_can_read: true,
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
    fn GetVolumeInformationByHandleW(
        file: *mut c_void,
        volume_name: *mut u16,
        volume_name_size: u32,
        serial_number: *mut u32,
        max_component_length: *mut u32,
        file_system_flags: *mut u32,
        file_system_name: *mut u16,
        file_system_name_size: u32,
    ) -> i32;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn GetSecurityInfo(
        handle: *mut c_void,
        object: u32,
        info: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut c_void,
        sacl: *mut *mut c_void,
        descriptor: *mut *mut c_void,
    ) -> u32;
    fn GetSecurityDescriptorLength(descriptor: *mut c_void) -> u32;
    fn IsValidSid(sid: *mut c_void) -> i32;
    fn GetLengthSid(sid: *mut c_void) -> u32;
}

pub(super) fn file_access(file: &File) -> io::Result<FileAccess> {
    access_of(file, Object::File)
}

/// Like [`file_access`], for a directory handle: entries that only pass
/// rights on to new children count as well.
pub(super) fn dir_access(dir: &File) -> io::Result<FileAccess> {
    access_of(dir, Object::Directory)
}

fn access_of(file: &File, object: Object) -> io::Result<FileAccess> {
    let handle = file.as_raw_handle();
    if !persistent_acls(handle)? {
        return Ok(UNENFORCED);
    }
    let descriptor = Descriptor::of(handle)?;
    let user = current_user_sid()?;
    evaluate(descriptor.bytes()?, &user, object)
}

/// What the security descriptor belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Object {
    /// A file: only entries that apply to the file itself count.
    File,
    /// A directory: entries that are inherited by new files and
    /// sub-directories count too, because whatever the application
    /// creates inside would grant those rights. `CREATOR OWNER` in an
    /// inherited entry is replaced by the creator, so it is trusted.
    Directory,
}

/// Whether the volume holding `handle` keeps access control lists.
fn persistent_acls(handle: *mut c_void) -> io::Result<bool> {
    let mut flags = 0u32;
    // SAFETY: `handle` is borrowed from a live `File`; every output we do
    // not want is null with a zero size, which the API allows; `flags` is
    // a valid out-pointer.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            handle,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut flags,
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(flags & FILE_PERSISTENT_ACLS != 0)
}

/// The current user's SID, copied out of the process token.
fn current_user_sid() -> io::Result<Vec<u8>> {
    super::private_dir::with_current_user_sid(|sid| {
        // SAFETY: `sid` points to the SID in the token buffer, alive for
        // the whole closure.
        if unsafe { IsValidSid(sid) } == 0 {
            return Err(malformed());
        }
        // SAFETY: `sid` is a valid SID (checked above).
        let len = unsafe { GetLengthSid(sid) };
        let len = usize::try_from(len).map_err(|_| malformed())?;
        // SAFETY: a valid SID occupies exactly `GetLengthSid` bytes, all
        // inside the live token buffer.
        let bytes = unsafe { std::slice::from_raw_parts(sid.cast::<u8>().cast_const(), len) };
        Ok(bytes.to_vec())
    })
}

/// A security descriptor allocated by the system, freed on drop on every
/// path.
struct Descriptor(*mut c_void);

impl Descriptor {
    /// The owner and DACL of the object behind `handle`. Needs
    /// `READ_CONTROL` on the handle, which `File::open` and `File::create`
    /// both request.
    fn of(handle: *mut c_void) -> io::Result<Self> {
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        // SAFETY: `handle` is borrowed from a live `File`; the owner,
        // group, DACL, and SACL out-pointers may be null, as documented,
        // because only the descriptor itself is wanted; `descriptor` is a
        // valid out-pointer.
        let status = unsafe {
            GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut descriptor,
            )
        };
        if status != 0 {
            // Error codes are small positive numbers; one that does not fit
            // an `i32` is reported without its code.
            return Err(i32::try_from(status).map_or_else(
                |_| io::Error::other("GetSecurityInfo failed"),
                io::Error::from_raw_os_error,
            ));
        }
        if descriptor.is_null() {
            return Err(io::Error::other("GetSecurityInfo returned no descriptor"));
        }
        Ok(Self(descriptor))
    }

    /// The whole descriptor as bytes. Refuses an absolute descriptor,
    /// whose parts may live elsewhere in memory.
    fn bytes(&self) -> io::Result<&[u8]> {
        // SAFETY: `self.0` is a non-null security descriptor from
        // `GetSecurityInfo`; every descriptor starts with a revision byte,
        // a padding byte, and a 16-bit control field, so these four bytes
        // are readable. `read_unaligned` makes no alignment assumption.
        let control = unsafe { self.0.cast::<u8>().add(2).cast::<u16>().read_unaligned() };
        if control & SE_SELF_RELATIVE == 0 {
            return Err(malformed());
        }
        // SAFETY: `self.0` is a valid descriptor, as above.
        let len = unsafe { GetSecurityDescriptorLength(self.0) };
        let len = usize::try_from(len).map_err(|_| malformed())?;
        // SAFETY: a self-relative descriptor is one contiguous block that
        // holds its header and every part it references, `len` bytes in
        // all, written by the system and alive until `drop`. The slice
        // borrows `self`, so it cannot outlive the allocation.
        Ok(unsafe { std::slice::from_raw_parts(self.0.cast::<u8>().cast_const(), len) })
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: `self.0` was allocated by `GetSecurityInfo` with
        // `LocalAlloc`, as documented, and is freed exactly once, here.
        // LocalFree returns null on success; there is nothing to do if it
        // fails.
        let _freed = unsafe { LocalFree(self.0) };
    }
}

/// Reads a self-relative security descriptor and decides, for `user`, who
/// else can change or read the file. Every read is bounds-checked;
/// anything out of bounds or inconsistent is an `InvalidData` error.
fn evaluate(descriptor: &[u8], user: &[u8], object: Object) -> io::Result<FileAccess> {
    if descriptor.len() < DESCRIPTOR_HEADER || descriptor.first() != Some(&1) {
        return Err(malformed());
    }
    let control = u16_at(descriptor, 2)?;
    if control & SE_SELF_RELATIVE == 0 {
        return Err(malformed());
    }
    let trusted = |sid: &[u8]| sid == user || sid == SYSTEM || sid == ADMINISTRATORS;

    let owner_offset = offset_at(descriptor, 4)?;
    let owner_trusted = match owner_offset {
        0 => false,
        at => trusted(sid_at(descriptor, at)?),
    };

    let dacl_offset = offset_at(descriptor, 16)?;
    if control & SE_DACL_PRESENT == 0 || dacl_offset == 0 {
        // No DACL, or a NULL DACL: the system grants everyone everything.
        return Ok(FileAccess {
            enforced: true,
            owner_trusted,
            others_can_write: true,
            others_can_read: true,
        });
    }

    let others = granted_to_others(descriptor, dacl_offset, object, |sid, inherit_only| {
        // OWNER RIGHTS stands for the owner, so it is as trusted as the
        // owner is. CREATOR OWNER in an entry that only exists to be
        // inherited becomes the creator of each child.
        if sid == OWNER_RIGHTS {
            owner_trusted
        } else if inherit_only && sid == CREATOR_OWNER {
            true
        } else {
            trusted(sid)
        }
    })?;
    Ok(FileAccess {
        enforced: true,
        owner_trusted,
        others_can_write: others & WRITE_RIGHTS != 0,
        others_can_read: others & READ_RIGHTS != 0,
    })
}

/// The union of the rights the ACL at `offset` grants to accounts that
/// `trusted` does not accept. Deny entries are ignored, so the answer can
/// only overstate what others can do, never understate it.
fn granted_to_others(
    descriptor: &[u8],
    offset: usize,
    object: Object,
    trusted: impl Fn(&[u8], bool) -> bool,
) -> io::Result<u32> {
    let rest = descriptor.get(offset..).ok_or_else(malformed)?;
    let size = usize::from(u16_at(rest, 2)?);
    let count = u16_at(rest, 4)?;
    if size < ACL_HEADER {
        return Err(malformed());
    }
    let acl = rest.get(..size).ok_or_else(malformed)?;
    let mut others = 0u32;
    let mut at = ACL_HEADER;
    for _ in 0..count {
        let header = acl.get(at..at + ACE_HEADER).ok_or_else(malformed)?;
        let (kind, flags) = (header[0], header[1]);
        let ace_size = usize::from(u16_at(header, 2)?);
        if ace_size < ACE_HEADER {
            return Err(malformed());
        }
        let end = at.checked_add(ace_size).ok_or_else(malformed)?;
        let ace = acl.get(at..end).ok_or_else(malformed)?;
        at = end;
        let inherit_only = flags & INHERIT_ONLY_ACE != 0;
        if inherit_only && object == Object::File {
            continue;
        }
        match kind {
            ACCESS_ALLOWED_ACE_TYPE | ACCESS_ALLOWED_CALLBACK_ACE_TYPE => {
                let mask = u32_at(ace, ACE_MASK)?;
                if !trusted(sid_at(ace, ACE_SID)?, inherit_only) {
                    others |= mask;
                }
            }
            ACCESS_ALLOWED_COMPOUND_ACE_TYPE
            | ACCESS_ALLOWED_OBJECT_ACE_TYPE
            | ACCESS_ALLOWED_CALLBACK_OBJECT_ACE_TYPE => others |= u32_at(ace, ACE_MASK)?,
            // Deny, audit, and label entries grant nothing.
            _ => {}
        }
    }
    Ok(others)
}

/// The SID that starts at `offset`, exactly as long as its sub-authority
/// count says.
fn sid_at(bytes: &[u8], offset: usize) -> io::Result<&[u8]> {
    let rest = bytes.get(offset..).ok_or_else(malformed)?;
    let (revision, count) = match rest {
        [revision, count, ..] => (*revision, *count),
        _ => return Err(malformed()),
    };
    if revision != 1 || count > MAX_SUB_AUTHORITIES {
        return Err(malformed());
    }
    rest.get(..8 + 4 * usize::from(count)).ok_or_else(malformed)
}

/// A 32-bit offset field of the descriptor header.
fn offset_at(bytes: &[u8], at: usize) -> io::Result<usize> {
    usize::try_from(u32_at(bytes, at)?).map_err(|_| malformed())
}

fn u16_at(bytes: &[u8], at: usize) -> io::Result<u16> {
    match bytes.get(at..at.checked_add(2).ok_or_else(malformed)?) {
        Some(&[a, b]) => Ok(u16::from_le_bytes([a, b])),
        _ => Err(malformed()),
    }
}

fn u32_at(bytes: &[u8], at: usize) -> io::Result<u32> {
    match bytes.get(at..at.checked_add(4).ok_or_else(malformed)?) {
        Some(&[a, b, c, d]) => Ok(u32::from_le_bytes([a, b, c, d])),
        _ => Err(malformed()),
    }
}

fn malformed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "the file's security descriptor is malformed",
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};

    use super::*;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
        fn GetSecurityDescriptorDacl(
            descriptor: *mut c_void,
            present: *mut i32,
            dacl: *mut *mut c_void,
            defaulted: *mut i32,
        ) -> i32;
        fn SetSecurityInfo(
            handle: *mut c_void,
            object: u32,
            info: u32,
            owner: *mut c_void,
            group: *mut c_void,
            dacl: *mut c_void,
            sacl: *mut c_void,
        ) -> u32;
        fn ConvertSidToStringSidW(sid: *mut c_void, string: *mut *mut u16) -> i32;
    }

    const SDDL_REVISION_1: u32 = 1;
    const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
    const READ_CONTROL: u32 = 0x0002_0000;
    const WRITE_DAC: u32 = 0x0004_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;

    /// `S-1-5-21-1-2-3-1001`, a made-up local account.
    const ALICE: &str = "S-1-5-21-1-2-3-1001";
    const ALICE_BYTES: [u8; 28] = [
        1, 5, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 0xE9, 0x03, 0, 0,
    ];

    /// The self-relative descriptor the system builds from `sddl`.
    fn descriptor_bytes(sddl: &str) -> Vec<u8> {
        let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        let mut size = 0u32;
        // SAFETY: `wide` is NUL-terminated; both out-pointers are valid.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &raw mut descriptor,
                &raw mut size,
            )
        };
        assert_ne!(ok, 0, "bad SDDL {sddl}");
        let owned = Descriptor(descriptor);
        let bytes = owned.bytes().unwrap().to_vec();
        assert_eq!(bytes.len(), size as usize);
        bytes
    }

    fn check(sddl: &str) -> FileAccess {
        evaluate(&descriptor_bytes(sddl), &ALICE_BYTES, Object::File).unwrap()
    }

    fn check_dir(sddl: &str) -> FileAccess {
        evaluate(&descriptor_bytes(sddl), &ALICE_BYTES, Object::Directory).unwrap()
    }

    fn sid_string(bytes: &[u8]) -> String {
        let mut copy = bytes.to_vec();
        let mut string: *mut u16 = std::ptr::null_mut();
        // SAFETY: `copy` holds a complete SID; `string` is a valid
        // out-pointer.
        let ok = unsafe { ConvertSidToStringSidW(copy.as_mut_ptr().cast(), &raw mut string) };
        assert_ne!(ok, 0);
        let mut units = Vec::new();
        for i in 0..256 {
            // SAFETY: NUL-terminated by the system; SIDs are short.
            let unit = unsafe { *string.add(i) };
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        // SAFETY: allocated by the system with `LocalAlloc`.
        let _freed = unsafe { LocalFree(string.cast()) };
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn test_well_known_sid_bytes_match_their_string_forms() {
        assert_eq!(sid_string(&SYSTEM), "S-1-5-18");
        assert_eq!(sid_string(&ADMINISTRATORS), "S-1-5-32-544");
        assert_eq!(sid_string(&OWNER_RIGHTS), "S-1-3-4");
        assert_eq!(sid_string(&ALICE_BYTES), ALICE);
    }

    #[test]
    fn test_current_user_sid_matches_the_token_string() {
        let bytes = current_user_sid().unwrap();
        let string = super::super::private_dir::current_user_sid().unwrap();
        assert_eq!(sid_string(&bytes), string);
    }

    #[test]
    fn test_evaluate_private_dacl_reports_no_others() {
        for owner in [ALICE, "SY", "BA"] {
            let a = check(&format!(
                "O:{owner}D:P(A;;FA;;;{ALICE})(A;;FA;;;SY)(A;;FA;;;BA)"
            ));
            assert!(a.enforced && a.owner_trusted, "{owner}: {a:?}");
            assert!(!a.others_can_write && !a.others_can_read, "{owner}: {a:?}");
        }
    }

    #[test]
    fn test_evaluate_untrusted_owner_is_reported() {
        let a = check(&format!("O:WDD:P(A;;FA;;;{ALICE})"));
        assert!(!a.owner_trusted, "{a:?}");
        assert!(!a.others_can_write && !a.others_can_read, "{a:?}");
        // No owner at all is not a trusted owner either.
        let a = check(&format!("D:P(A;;FA;;;{ALICE})"));
        assert!(!a.owner_trusted, "{a:?}");
    }

    #[test]
    fn test_evaluate_write_and_read_rights_are_told_apart() {
        let a = check(&format!("O:{ALICE}D:P(A;;FA;;;{ALICE})(A;;FW;;;WD)"));
        assert!(a.others_can_write && !a.others_can_read, "{a:?}");
        let a = check(&format!("O:{ALICE}D:P(A;;FA;;;{ALICE})(A;;FR;;;BU)"));
        assert!(!a.others_can_write && a.others_can_read, "{a:?}");
        // Execute and attribute reads are neither.
        let a = check(&format!("O:{ALICE}D:P(A;;FX;;;WD)"));
        assert!(!a.others_can_write && !a.others_can_read, "{a:?}");
    }

    #[test]
    fn test_evaluate_every_write_right_counts() {
        for right in [
            "WD", "WO", "SD", "GA", "GW", "0x2", "0x4", "0x10", "0x40", "0x100",
        ] {
            let a = check(&format!("O:{ALICE}D:P(A;;{right};;;AU)"));
            assert!(a.others_can_write, "{right}: {a:?}");
        }
        for right in ["GR", "GA", "0x1"] {
            let a = check(&format!("O:{ALICE}D:P(A;;{right};;;AU)"));
            assert!(a.others_can_read, "{right}: {a:?}");
        }
    }

    #[test]
    fn test_evaluate_null_and_missing_dacl_grant_everyone_everything() {
        for sddl in [
            format!("O:{ALICE}D:NO_ACCESS_CONTROL"),
            format!("O:{ALICE}"),
        ] {
            let a = check(&sddl);
            assert!(a.owner_trusted, "{sddl}: {a:?}");
            assert!(a.others_can_write && a.others_can_read, "{sddl}: {a:?}");
        }
    }

    #[test]
    fn test_evaluate_empty_dacl_grants_nobody() {
        let a = check(&format!("O:{ALICE}D:P"));
        assert!(!a.others_can_write && !a.others_can_read, "{a:?}");
    }

    #[test]
    fn test_evaluate_ignores_inherit_only_and_deny_entries() {
        let a = check(&format!(
            "O:{ALICE}D:P(A;OICIIO;FA;;;WD)(D;;FA;;;{ALICE})(A;;FA;;;{ALICE})"
        ));
        assert!(!a.others_can_write && !a.others_can_read, "{a:?}");
        // A deny entry never hides an allow entry for someone else.
        let a = check(&format!("O:{ALICE}D:P(D;;FA;;;WD)(A;;FA;;;WD)"));
        assert!(a.others_can_write && a.others_can_read, "{a:?}");
    }

    #[test]
    fn test_evaluate_directory_counts_inherited_entries() {
        // Inherit-only entries grant nothing on a file, but on a directory
        // they decide what every new child grants.
        let sddl = format!("O:{ALICE}D:P(A;;FA;;;{ALICE})(A;OICIIO;FW;;;BU)");
        let a = check(&sddl);
        assert!(!a.others_can_write, "{a:?}");
        let a = check_dir(&sddl);
        assert!(a.others_can_write && !a.others_can_read, "{a:?}");
        let a = check_dir(&format!("O:{ALICE}D:P(A;;FA;;;{ALICE})(A;OIIO;FR;;;WD)"));
        assert!(!a.others_can_write && a.others_can_read, "{a:?}");
    }

    #[test]
    fn test_evaluate_directory_trusts_inherited_creator_owner_only() {
        // The %ProgramData% pattern: CREATOR OWNER full control, inherited.
        let a = check_dir(&format!(
            "O:{ALICE}D:P(A;;FA;;;{ALICE})(A;OICIIO;GA;;;CO)(A;;FA;;;SY)"
        ));
        assert!(!a.others_can_write && !a.others_can_read, "{a:?}");
        // On the directory itself CREATOR OWNER is nobody in particular.
        let a = check_dir(&format!("O:{ALICE}D:P(A;;FA;;;CO)"));
        assert!(a.others_can_write, "{a:?}");
        // CREATOR GROUP becomes the creator's group: other accounts.
        let a = check_dir(&format!("O:{ALICE}D:P(A;OICIIO;GA;;;CG)"));
        assert!(a.others_can_write && a.others_can_read, "{a:?}");
    }

    #[test]
    fn test_evaluate_program_data_default_is_open_to_users() {
        // The default ACL of %ProgramData%, which every directory created
        // there with plain inheritance receives.
        let a = check_dir(
            "O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICIIO;GA;;;CO)\
             (A;OICI;0x1200a9;;;BU)(A;CI;0x116;;;BU)",
        );
        assert!(
            a.owner_trusted && a.others_can_write && a.others_can_read,
            "{a:?}"
        );
    }

    #[test]
    fn test_evaluate_owner_rights_follow_the_owner() {
        let a = check(&format!("O:{ALICE}D:P(A;;FA;;;OW)"));
        assert!(!a.others_can_write && !a.others_can_read, "{a:?}");
        let a = check("O:WDD:P(A;;FA;;;OW)");
        assert!(a.others_can_write && a.others_can_read, "{a:?}");
    }

    #[test]
    fn test_evaluate_creator_owner_and_users_are_others() {
        for trustee in ["CO", "BU", "AU", "WD", "S-1-5-21-1-2-3-1002"] {
            let a = check(&format!("O:{ALICE}D:P(A;;FA;;;{trustee})"));
            assert!(a.others_can_write && a.others_can_read, "{trustee}: {a:?}");
        }
    }

    #[test]
    fn test_evaluate_callback_ace_counts_like_a_plain_one() {
        // A conditional entry is a callback ACE; it may grant access.
        let a = check(&format!(
            "O:{ALICE}D:P(XA;;FW;;;WD;(Member_of {{SID(BA)}}))"
        ));
        assert!(a.others_can_write, "{a:?}");
    }

    #[test]
    fn test_evaluate_rejects_every_truncation() {
        let full = descriptor_bytes(&format!("O:{ALICE}D:P(A;;FA;;;{ALICE})(A;;FW;;;WD)"));
        assert!(evaluate(&full, &ALICE_BYTES, Object::File).is_ok());
        for len in 0..full.len() {
            let err = evaluate(&full[..len], &ALICE_BYTES, Object::File);
            assert!(
                err.as_ref()
                    .is_err_and(|e| e.kind() == io::ErrorKind::InvalidData),
                "length {len}: {err:?}"
            );
        }
    }

    #[test]
    fn test_evaluate_rejects_inconsistent_fields() {
        let full = descriptor_bytes(&format!("O:{ALICE}D:P(A;;FA;;;WD)"));
        let dacl = u32_at(&full, 16).unwrap() as usize;
        let owner = u32_at(&full, 4).unwrap() as usize;
        let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
        let mut absolute = full.clone();
        absolute[3] &= 0x7F;
        cases.push(("absolute form", absolute));
        let mut revision = full.clone();
        revision[0] = 2;
        cases.push(("descriptor revision", revision));
        let mut acl_size = full.clone();
        acl_size[dacl + 2..dacl + 4].copy_from_slice(&u16::MAX.to_le_bytes());
        cases.push(("ACL size past the end", acl_size));
        let mut ace_count = full.clone();
        ace_count[dacl + 4] = 9;
        cases.push(("more ACEs than fit", ace_count));
        let mut ace_size = full.clone();
        ace_size[dacl + 8 + 2..dacl + 8 + 4].copy_from_slice(&0u16.to_le_bytes());
        cases.push(("zero ACE size", ace_size));
        let mut sid_count = full.clone();
        sid_count[owner + 1] = 16;
        cases.push(("too many sub-authorities", sid_count));
        let mut far = full.clone();
        far[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        cases.push(("DACL offset past the end", far));
        for (what, bytes) in cases {
            let err = evaluate(&bytes, &ALICE_BYTES, Object::File);
            assert!(
                err.as_ref()
                    .is_err_and(|e| e.kind() == io::ErrorKind::InvalidData),
                "{what}: {err:?}"
            );
        }
    }

    // --- against real files ----------------------------------------------

    /// A fresh private directory, created like `App::ensure_private` does.
    fn private_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("path-lib-access-{label}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&dir));
        super::super::private_dir::create(&dir, false).unwrap();
        dir
    }

    /// Replaces the file's DACL with a protected one from `sddl`, or with
    /// a NULL DACL when `sddl` is `None`.
    fn set_dacl(path: &Path, sddl: Option<&str>) {
        let file = std::fs::OpenOptions::new()
            .access_mode(READ_CONTROL | WRITE_DAC)
            .open(path)
            .unwrap();
        let mut descriptor: *mut c_void = std::ptr::null_mut();
        let mut dacl: *mut c_void = std::ptr::null_mut();
        if let Some(sddl) = sddl {
            let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
            // SAFETY: as in `descriptor_bytes`.
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    SDDL_REVISION_1,
                    &raw mut descriptor,
                    std::ptr::null_mut(),
                )
            };
            assert_ne!(ok, 0, "bad SDDL {sddl}");
            let (mut present, mut defaulted) = (0i32, 0i32);
            // SAFETY: `descriptor` is valid; all out-pointers are valid.
            let ok = unsafe {
                GetSecurityDescriptorDacl(
                    descriptor,
                    &raw mut present,
                    &raw mut dacl,
                    &raw mut defaulted,
                )
            };
            assert!(ok != 0 && present != 0 && !dacl.is_null());
        }
        // SAFETY: the handle has WRITE_DAC; `dacl` is null (a NULL DACL,
        // as documented) or points into `descriptor`, which is still alive.
        let status = unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null_mut(),
            )
        };
        if !descriptor.is_null() {
            // SAFETY: allocated by the conversion with `LocalAlloc`.
            let _freed = unsafe { LocalFree(descriptor) };
        }
        assert_eq!(status, 0, "SetSecurityInfo failed");
    }

    fn access_of(path: &Path) -> FileAccess {
        let a = file_access(&File::open(path).unwrap()).unwrap();
        // The temp directory is on NTFS here; without that the other
        // checks would pass for the wrong reason.
        assert!(a.enforced, "temp volume does not keep ACLs: {a:?}");
        a
    }

    #[test]
    fn test_file_access_in_a_private_dir_has_no_others() {
        let dir = private_dir("private");
        let path = dir.join("app.conf");
        std::fs::write(&path, "port = 7000\n").unwrap();
        let a = access_of(&path);
        assert!(a.owner_trusted, "{a:?}");
        assert!(!a.others_can_write && !a.others_can_read, "{a:?}");
        // A writable handle answers the same.
        let writable = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        assert_eq!(file_access(&writable).unwrap(), a);
        drop(writable);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_file_access_sees_everyone_write() {
        let dir = private_dir("everyone-write");
        let path = dir.join(".env");
        std::fs::write(&path, "KEY=1\n").unwrap();
        let user = super::super::private_dir::current_user_sid().unwrap();
        set_dacl(&path, Some(&format!("D:P(A;;FA;;;{user})(A;;FW;;;WD)")));
        let a = access_of(&path);
        assert!(a.owner_trusted, "{a:?}");
        assert!(a.others_can_write && !a.others_can_read, "{a:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_file_access_sees_everyone_read() {
        let dir = private_dir("everyone-read");
        let path = dir.join("secret.key");
        std::fs::write(&path, "k").unwrap();
        let user = super::super::private_dir::current_user_sid().unwrap();
        set_dacl(&path, Some(&format!("D:P(A;;FA;;;{user})(A;;FR;;;WD)")));
        let a = access_of(&path);
        assert!(!a.others_can_write && a.others_can_read, "{a:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_file_access_null_dacl_reports_both() {
        let dir = private_dir("null-dacl");
        let path = dir.join("app.conf");
        std::fs::write(&path, "x").unwrap();
        set_dacl(&path, None);
        let a = access_of(&path);
        assert!(a.others_can_write && a.others_can_read, "{a:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_file_access_without_read_control_fails() {
        let dir = private_dir("no-read-control");
        let path = dir.join("app.conf");
        std::fs::write(&path, "x").unwrap();
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .open(&path)
            .unwrap();
        let err = file_access(&file).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{err:?}");
        drop(file);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_unenforced_answer_is_the_unsafe_one() {
        let unsafe_answer = FileAccess {
            enforced: false,
            owner_trusted: false,
            others_can_write: true,
            others_can_read: true,
        };
        assert_eq!(UNENFORCED, unsafe_answer);
    }
}
