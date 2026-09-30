//! Owner-only files on Windows.
//!
//! `buzz-agent` forbids `unsafe`, but keeping its OAuth token cache private on
//! Windows takes Win32 security calls. They live here behind a safe API so that
//! crate keeps its guarantee. Unix gets the same property from file modes in
//! std, so this crate is empty there.
#![cfg(windows)]
#![deny(unsafe_code)]

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    LocalFree, ERROR_SUCCESS, GENERIC_WRITE, INVALID_HANDLE_VALUE, WIN32_ERROR,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetSecurityDescriptorDacl, GetTokenInformation, TokenOwner,
    DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    SECURITY_ATTRIBUTES, TOKEN_OWNER, TOKEN_QUERY,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, WRITE_DAC,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Full access for the file's owner and nobody else; `P` blocks inherited ACEs.
const OWNER_ONLY: &str = "D:P(A;;FA;;;OW)";

/// Create `path` for writing, owner-only from the moment it exists.
///
/// Fails if `path` exists. The DACL is set atomically at creation, so no other
/// user can open the file even briefly. Read, write and delete sharing match
/// std, so another process can still rename a new file over this path.
#[allow(unsafe_code)]
pub fn create_new(path: &Path) -> io::Result<File> {
    let descriptor = owner_only()?;
    let path = verbatim(path)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    // SAFETY: `path` is NUL-terminated and `attributes` points at a descriptor
    // that outlives the call.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `handle` is a valid file handle that nothing else owns.
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// Open an existing owner-only file for reading.
///
/// Does not follow a final symlink or junction, and refuses anything but a
/// regular file owned by this process's default owner, which is the owner
/// [`create_new`] assigns. It then re-applies the owner-only DACL on the open
/// handle, so a file with inherited permissions is tightened before use.
pub fn open_owned(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .access_mode(FILE_GENERIC_READ | WRITE_DAC)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    if !owned_by_current_process(&file)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "file is not owned by the current user",
        ));
    }
    restrict_to_owner(&file)?;
    Ok(file)
}

/// Memory Windows allocated with `LocalAlloc`, freed on drop.
struct Local(*mut c_void);

impl Drop for Local {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: Windows allocated this pointer, and only this drop frees it.
        unsafe { LocalFree(self.0) };
    }
}

/// [`OWNER_ONLY`] as a security descriptor.
#[allow(unsafe_code)]
fn owner_only() -> io::Result<Local> {
    let sddl: Vec<u16> = OWNER_ONLY.encode_utf16().chain(Some(0)).collect();
    let mut descriptor = null_mut();
    // SAFETY: `sddl` is NUL-terminated; on success Windows allocates `descriptor`.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Local(descriptor))
}

/// Replace the DACL of a handle opened with `WRITE_DAC` by [`OWNER_ONLY`].
#[allow(unsafe_code)]
fn restrict_to_owner(file: &File) -> io::Result<()> {
    let descriptor = owner_only()?;
    let (mut present, mut dacl, mut defaulted) = (0, null_mut(), 0);
    // SAFETY: `descriptor` is valid, and `dacl` points into it.
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the handle is open, and `dacl` stays valid while `descriptor` lives.
    win32(unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            dacl,
            null(),
        )
    })
}

/// Whether the file's owner is this process token's default owner.
#[allow(unsafe_code)]
fn owned_by_current_process(file: &File) -> io::Result<bool> {
    let (mut owner, mut descriptor) = (null_mut(), null_mut());
    // SAFETY: the handle is open with `READ_CONTROL`; on success Windows
    // allocates `descriptor` and points `owner` into it.
    win32(unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    })?;
    let _descriptor = Local(descriptor);

    let mut token = null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle; on success `token`
    // is a new handle.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `token` is a valid handle that nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut len = 0;
    // SAFETY: a null, zero-length buffer only queries the required size.
    unsafe { GetTokenInformation(token.as_raw_handle(), TokenOwner, null_mut(), 0, &mut len) };
    // `usize` elements keep the buffer aligned for `TOKEN_OWNER`'s pointer.
    let mut buffer = vec![0usize; (len as usize).div_ceil(size_of::<usize>())];
    // SAFETY: `buffer` is aligned and at least `len` bytes long.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenOwner,
            buffer.as_mut_ptr().cast(),
            len,
            &mut len,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Windows wrote a `TOKEN_OWNER` whose SID lives in `buffer`, and
    // `owner` lives in `_descriptor`; both outlive the comparison.
    Ok(unsafe { EqualSid(owner, (*buffer.as_ptr().cast::<TOKEN_OWNER>()).Owner) } != 0)
}

fn win32(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

/// `path` as a NUL-terminated wide string. Drive paths get the `\\?\` prefix
/// that std's own file APIs add, so long paths work here too.
fn verbatim(path: &Path) -> io::Result<Vec<u16>> {
    let wide: Vec<u16> = std::path::absolute(path)?
        .as_os_str()
        .encode_wide()
        .collect();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    let drive = matches!(wide.as_slice(), [_, colon, slash, ..]
        if *colon == u16::from(b':') && *slash == u16::from(b'\\'));
    let prefix = if drive { r"\\?\" } else { "" };
    Ok(prefix.encode_utf16().chain(wide).chain(Some(0)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::process::Command;
    use windows_sys::Win32::Security::Authorization::ConvertSecurityDescriptorToStringSecurityDescriptorW;

    /// The file's DACL as SDDL, such as `D:P(A;;FA;;;OW)`.
    #[allow(unsafe_code)]
    fn dacl(file: &File) -> String {
        let mut descriptor = null_mut();
        // SAFETY: the handle is open; on success Windows allocates `descriptor`.
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        };
        win32(status).unwrap();
        let descriptor = Local(descriptor);
        let (mut sddl, mut len) = (null_mut(), 0);
        // SAFETY: `descriptor` is valid; on success Windows allocates `sddl`
        // holding `len` UTF-16 units.
        let ok = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor.0,
                SDDL_REVISION_1,
                DACL_SECURITY_INFORMATION,
                &mut sddl,
                &mut len,
            )
        };
        assert_ne!(ok, 0, "{}", io::Error::last_os_error());
        let _sddl = Local(sddl.cast());
        // SAFETY: `sddl` holds `len` UTF-16 units.
        let units = unsafe { std::slice::from_raw_parts(sddl, len as usize) };
        String::from_utf16_lossy(units)
            .trim_end_matches('\0')
            .to_owned()
    }

    fn assert_owner_only(file: &File) {
        let sddl = dacl(file);
        let (flags, aces) = sddl
            .strip_prefix("D:")
            .and_then(|rest| rest.split_once('('))
            .unwrap_or_else(|| panic!("unexpected DACL {sddl}"));
        assert!(flags.contains('P'), "DACL is not protected: {sddl}");
        assert_eq!(aces, "A;;FA;;;OW)", "DACL is not owner-only: {sddl}");
    }

    fn read(mut file: File) -> String {
        let mut body = String::new();
        file.read_to_string(&mut body).unwrap();
        body
    }

    #[test]
    fn create_new_is_owner_only_and_exclusive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        let mut file = create_new(&path).unwrap();
        file.write_all(b"secret").unwrap();
        assert_owner_only(&file);
        drop(file);

        let err = create_new(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read(open_owned(&path).unwrap()), "secret");
    }

    #[test]
    fn create_new_accepts_long_paths() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("a".repeat(120)).join("b".repeat(120));
        std::fs::create_dir_all(&parent).unwrap();
        let path = parent.join("token.json");
        create_new(&path).unwrap();
        assert_owner_only(&open_owned(&path).unwrap());
    }

    #[test]
    fn open_owned_tightens_inherited_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        std::fs::write(&path, "legacy").unwrap();
        let file = open_owned(&path).unwrap();
        assert_owner_only(&file);
        assert_eq!(read(file), "legacy");
    }

    fn icacls(path: &Path, args: &[&str]) {
        let status = Command::new("icacls")
            .arg(path)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "icacls {args:?} failed; run elevated");
    }

    #[test]
    #[ignore = "needs an elevated shell; CI runs it"]
    fn open_owned_refuses_a_file_owned_by_someone_else() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        std::fs::write(&path, "planted").unwrap();
        // Everyone may read and rewrite it, but SYSTEM, not us, owns it.
        icacls(&path, &["/grant", "*S-1-1-0:F"]);
        icacls(&path, &["/setowner", "*S-1-5-18"]);
        let err = open_owned(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    #[ignore = "needs an elevated shell; CI runs it"]
    fn open_owned_does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere.json");
        let link = dir.path().join("token.json");
        std::fs::write(&target, "elsewhere").unwrap();
        std::os::windows::fs::symlink_file(&target, &link).unwrap();
        let err = open_owned(&link).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
