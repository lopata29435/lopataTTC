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

/// Re-launch a binary with elevated privileges via the system's standard
/// graphical prompt. On Linux this uses `pkexec` (PolicyKit), on macOS
/// `osascript … with administrator privileges` (the native authorization
/// dialog). Both spawn a fresh process; the caller is expected to exit afterwards
/// if it wanted to "restart as admin".
#[cfg(target_os = "linux")]
pub fn run_elevated(exe: &Path, args: &str, working_dir: Option<&Path>) -> Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut cmd = Command::new("pkexec");
    // pkexec resets the environment to a tiny safe-list by default. We have
    // to re-inject DISPLAY / WAYLAND_DISPLAY / XAUTHORITY / XDG_RUNTIME_DIR so
    // the elevated binary can actually connect to the user's GUI session.
    // (`env` is a tiny POSIX utility that sets env vars then execs the next arg.)
    cmd.arg("env");
    for var in [
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                cmd.arg(format!("{}={}", var, v));
            }
        }
    }
    cmd.arg(exe);
    if !args.trim().is_empty() {
        let parts = shlex::split(args).ok_or_else(|| anyhow!("malformed args: {}", args))?;
        for p in parts {
            cmd.arg(p);
        }
    }
    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }
    // Detach: new session means our SIGHUP on exit won't kill the child.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    cmd.spawn()
        .map_err(|e| anyhow!("pkexec spawn failed: {} — установлен ли pkexec/PolicyKit?", e))?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn run_elevated(exe: &Path, args: &str, _working_dir: Option<&Path>) -> Result<()> {
    use std::process::Command;
    // AppleScript: `do shell script "<cmd>" with administrator privileges` shows
    // the native authorization dialog and runs the command as root.
    // We append ` &` so the spawned binary detaches and osascript returns
    // immediately — otherwise the AppleScript would block until our GUI exits.
    let mut cmd_str = shell_quote(&exe.display().to_string());
    if !args.trim().is_empty() {
        cmd_str.push(' ');
        cmd_str.push_str(args);
    }
    // Detach the child so AppleScript doesn't wait on it.
    cmd_str.push_str(" > /dev/null 2>&1 &");
    // Escape for embedding inside a double-quoted AppleScript string literal.
    let escaped = cmd_str.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        escaped
    );
    Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .spawn()
        .map_err(|e| anyhow!("osascript spawn failed: {}", e))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn shell_quote(s: &str) -> String {
    // Single-quote and escape any internal single-quotes; safe under sh.
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn run_elevated(_exe: &Path, _args: &str, _working_dir: Option<&Path>) -> Result<()> {
    Err(anyhow!("elevation not supported on this platform"))
}

#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
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
        )
        .is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

#[cfg(unix)]
pub fn is_elevated() -> bool {
    // On Unix, "elevated" = running as root (uid 0).
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(any(windows, unix)))]
pub fn is_elevated() -> bool {
    false
}
