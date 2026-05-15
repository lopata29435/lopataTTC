use anyhow::{anyhow, Result};
use std::path::Path;

#[cfg(windows)]
pub fn run_elevated(exe: &Path, args: &str, working_dir: Option<&Path>) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::{ShellExecuteW, SE_ERR_ACCESSDENIED};
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

    fn to_wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    let verb: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();
    let file = to_wide(exe.as_os_str());
    let args_w: Vec<u16> = args.encode_utf16().chain(std::iter::once(0)).collect();
    let dir_owned;
    let dir_ptr: PCWSTR = if let Some(d) = working_dir {
        dir_owned = to_wide(d.as_os_str());
        PCWSTR(dir_owned.as_ptr())
    } else {
        PCWSTR::null()
    };

    let hinst = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(args_w.as_ptr()),
            dir_ptr,
            SW_HIDE,
        )
    };
    let code = hinst.0 as isize;
    if code <= 32 {
        if code == SE_ERR_ACCESSDENIED as isize {
            return Err(anyhow!("UAC denied (access denied)"));
        }
        return Err(anyhow!("ShellExecuteW failed: code {}", code));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn run_elevated(_exe: &Path, _args: &str, _working_dir: Option<&Path>) -> Result<()> {
    Err(anyhow!("elevation not supported on this platform"))
}

#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        ).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool { false }
