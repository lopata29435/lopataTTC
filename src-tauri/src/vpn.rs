use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex as AsyncMutex;

use crate::profiles::Profile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatePayload {
    pub state: ConnectionState,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub message: Option<String>,
    pub started_at: Option<i64>, // unix seconds
}

#[derive(Debug, Clone, Serialize)]
pub struct LogPayload {
    pub line: String,
    pub level: String, // "info" | "warn" | "error"
}

const MAX_LOG_LINES: usize = 500;

/// Per-connection control block. `alive` flips to false when the session ends
/// (either by user request or because the process died) so that auxiliary
/// tasks (log tail) know to stop. `stop_flag` is set for elevated Unix
/// sessions: creating that file asks the root-side wrapper script to terminate
/// the client — we can't signal a root process from an unprivileged GUI.
struct SessionCtl {
    alive: Arc<AtomicBool>,
    stop_flag: Option<PathBuf>,
}

pub struct VpnService {
    state: Arc<Mutex<StatePayload>>,
    log_buffer: Arc<Mutex<VecDeque<LogPayload>>>,
    child: Arc<AsyncMutex<Option<Child>>>,
    binary_path: Arc<Mutex<PathBuf>>,
    config_path: PathBuf,
    session: Mutex<Option<SessionCtl>>,
}

impl VpnService {
    pub fn new(binary_path: Arc<Mutex<PathBuf>>, config_path: PathBuf) -> Self {
        Self {
            state: Arc::new(Mutex::new(StatePayload {
                state: ConnectionState::Disconnected,
                profile_id: None,
                profile_name: None,
                message: None,
                started_at: None,
            })),
            log_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES))),
            child: Arc::new(AsyncMutex::new(None)),
            binary_path,
            config_path,
            session: Mutex::new(None),
        }
    }

    pub fn state(&self) -> StatePayload {
        self.state.lock().unwrap().clone()
    }

    pub fn logs(&self) -> Vec<LogPayload> {
        self.log_buffer.lock().unwrap().iter().cloned().collect()
    }

    pub fn binary_path(&self) -> PathBuf {
        self.binary_path.lock().unwrap().clone()
    }

    pub fn set_binary_path(&self, new_path: PathBuf) {
        *self.binary_path.lock().unwrap() = new_path;
    }

    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    #[cfg(unix)]
    fn run_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(std::env::temp_dir)
    }

    fn set_state(&self, app: &AppHandle, new: StatePayload) {
        {
            let mut s = self.state.lock().unwrap();
            *s = new.clone();
        }
        let _ = app.emit("vpn://state", &new);
    }

    fn push_log(&self, app: &AppHandle, line: String, level: &str) {
        let payload = LogPayload {
            line,
            level: level.into(),
        };
        {
            let mut buf = self.log_buffer.lock().unwrap();
            if buf.len() >= MAX_LOG_LINES {
                buf.pop_front();
            }
            buf.push_back(payload.clone());
        }
        let _ = app.emit("vpn://log", &payload);
    }

    pub async fn connect(&self, app: AppHandle, profile: Profile) -> Result<()> {
        // On Windows the whole GUI runs elevated (requireAdministrator manifest),
        // and the client inherits admin. If we somehow ended up unelevated,
        // there is no way to create the WinTUN adapter — fail early.
        #[cfg(windows)]
        if !crate::elevate::is_elevated() {
            anyhow::bail!(
                "Для подключения нужны права администратора. Перезапустите приложение от имени администратора."
            );
        }

        // shut down any existing process first
        self.disconnect_internal().await.ok();

        let binary_path = self.binary_path();
        if !binary_path.exists() {
            anyhow::bail!(
                "VPN-клиент не найден ({}). Дождитесь окончания загрузки клиента или нажмите «Проверить обновления» в Настройках.",
                binary_path.display()
            );
        }

        // write config file
        let toml_text = profile.to_client_toml();
        if let Some(parent) = self.config_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&self.config_path, toml_text)
            .with_context(|| format!("write config to {}", self.config_path.display()))?;
        // The config contains credentials — keep it private to the user.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                std::fs::set_permissions(&self.config_path, std::fs::Permissions::from_mode(0o600));
        }

        self.set_state(
            &app,
            StatePayload {
                state: ConnectionState::Connecting,
                profile_id: Some(profile.id.clone()),
                profile_name: Some(profile.name.clone()),
                message: Some(format!("Подключение к {}...", profile.hostname)),
                started_at: None,
            },
        );
        self.push_log(
            &app,
            format!(
                "=== Запуск клиента: профиль \"{}\" ({}) ===",
                profile.name, profile.hostname
            ),
            "info",
        );

        // On Unix the GUI itself runs unprivileged (running a WebKit GUI as
        // root is unsupported and crashes on most distros). Only the client
        // process is elevated, via pkexec (Linux) / osascript (macOS).
        #[cfg(unix)]
        let elevated_session = !crate::elevate::is_elevated();
        #[cfg(not(unix))]
        let elevated_session = false;

        let alive = Arc::new(AtomicBool::new(true));

        let direct_cmd = || {
            let mut cmd = Command::new(&binary_path);
            cmd.arg("--config").arg(&self.config_path);
            cmd
        };
        #[cfg(unix)]
        let (mut cmd, stop_flag, log_file) = if elevated_session {
            self.build_elevated_unix_command(&binary_path)?
        } else {
            (direct_cmd(), None, None)
        };
        #[cfg(not(unix))]
        let (mut cmd, stop_flag, log_file): (Command, Option<PathBuf>, Option<PathBuf>) =
            (direct_cmd(), None, None);

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| {
            if elevated_session {
                anyhow!(
                    "Не удалось запустить элевацию ({}). Установлен ли PolicyKit (pkexec)?",
                    e
                )
            } else {
                anyhow!("Не удалось запустить {}: {}", binary_path.display(), e)
            }
        })?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        {
            let mut slot = self.child.lock().await;
            *slot = Some(child);
        }
        {
            let mut session = self.session.lock().unwrap();
            *session = Some(SessionCtl {
                alive: alive.clone(),
                stop_flag: stop_flag.clone(),
            });
        }

        let state = self.state.clone();
        let log_buffer = self.log_buffer.clone();
        let profile_id = profile.id.clone();
        let profile_name = profile.name.clone();

        // stdout/stderr of the direct child (or of pkexec/osascript in the
        // elevated case — auth errors land on stderr there).
        if let Some(stdout) = stdout {
            spawn_pipe_reader(
                stdout,
                app.clone(),
                state.clone(),
                log_buffer.clone(),
                profile_id.clone(),
                profile_name.clone(),
                "info",
            );
        }
        if let Some(stderr) = stderr {
            spawn_pipe_reader(
                stderr,
                app.clone(),
                state.clone(),
                log_buffer.clone(),
                profile_id.clone(),
                profile_name.clone(),
                "warn",
            );
        }

        // Elevated sessions write the client log to a file (the wrapper script
        // redirects it) — tail that file and feed it through the same pipeline.
        if let Some(log_path) = log_file {
            let app2 = app.clone();
            let state2 = state.clone();
            let log_buffer2 = log_buffer.clone();
            let profile_id2 = profile_id.clone();
            let profile_name2 = profile_name.clone();
            let alive2 = alive.clone();
            tokio::spawn(async move {
                let mut pos: u64 = 0;
                let mut pending = String::new();
                loop {
                    let still_alive = alive2.load(Ordering::Relaxed);
                    if let Ok(data) = tokio::fs::read(&log_path).await {
                        if (data.len() as u64) > pos {
                            let chunk = String::from_utf8_lossy(&data[pos as usize..]).into_owned();
                            pos = data.len() as u64;
                            pending.push_str(&chunk);
                            while let Some(idx) = pending.find('\n') {
                                let line: String = pending.drain(..=idx).collect();
                                handle_log_line(
                                    &app2,
                                    &state2,
                                    &log_buffer2,
                                    &profile_id2,
                                    &profile_name2,
                                    line.trim_end(),
                                    "info",
                                );
                            }
                        }
                    }
                    if !still_alive {
                        // One final read happened above; flush any tail line.
                        if !pending.trim().is_empty() {
                            handle_log_line(
                                &app2,
                                &state2,
                                &log_buffer2,
                                &profile_id2,
                                &profile_name2,
                                pending.trim_end(),
                                "info",
                            );
                        }
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
            });
        }

        // Watcher: if process exits, transition to Disconnected/Error
        let child_slot = self.child.clone();
        let state_for_watcher = self.state.clone();
        let app_for_watcher = app.clone();
        let profile_id_w = profile_id.clone();
        let profile_name_w = profile_name.clone();
        let alive_w = alive.clone();
        tokio::spawn(async move {
            // Loop until process is taken away (disconnect) or it exits naturally
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                let mut guard = child_slot.lock().await;
                if let Some(child) = guard.as_mut() {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            *guard = None;
                            drop(guard);
                            alive_w.store(false, Ordering::Relaxed);
                            let was_connected = {
                                let s = state_for_watcher.lock().unwrap();
                                s.state == ConnectionState::Connected
                            };
                            let code = status.code().unwrap_or(-1);
                            // pkexec exit codes: 126 = user dismissed the auth
                            // dialog, 127 = not authorized / auth failed.
                            let auth_denied = elevated_session && (code == 126 || code == 127);
                            let message = if auth_denied {
                                "Авторизация отменена — для VPN нужны права администратора"
                                    .to_string()
                            } else {
                                format!("Клиент завершился (код {})", code)
                            };
                            let new = StatePayload {
                                state: if status.success() || was_connected {
                                    ConnectionState::Disconnected
                                } else {
                                    ConnectionState::Error
                                },
                                profile_id: Some(profile_id_w.clone()),
                                profile_name: Some(profile_name_w.clone()),
                                message: Some(message),
                                started_at: None,
                            };
                            {
                                let mut s = state_for_watcher.lock().unwrap();
                                *s = new.clone();
                            }
                            let _ = app_for_watcher.emit("vpn://state", &new);
                            break;
                        }
                        Ok(None) => continue,
                        Err(_) => break,
                    }
                } else {
                    // disconnect() emptied the slot
                    alive_w.store(false, Ordering::Relaxed);
                    break;
                }
            }
        });

        Ok(())
    }

    /// Build the pkexec/osascript command that runs the client elevated, plus
    /// the stop-flag and log-file paths used to control/observe it.
    ///
    /// The wrapper script runs as root: it starts the client with output
    /// redirected to the log file, then polls for the stop-flag file. The
    /// unprivileged GUI requests termination simply by creating that file.
    #[cfg(unix)]
    fn build_elevated_unix_command(
        &self,
        binary_path: &std::path::Path,
    ) -> Result<(Command, Option<PathBuf>, Option<PathBuf>)> {
        use std::os::unix::fs::PermissionsExt;

        let run_dir = self.run_dir();
        std::fs::create_dir_all(&run_dir).ok();
        let wrapper = run_dir.join("client-wrapper.sh");
        let log_file = run_dir.join("client.log");
        let stop_flag = run_dir.join("client.stop");

        const WRAPPER_SH: &str = r#"#!/bin/sh
# TrustTunnel GUI: runs the VPN client as root and watches for a stop request.
# Usage: client-wrapper.sh <client-binary> <config> <log-file> <stop-flag>
bin="$1"; cfg="$2"; log="$3"; flag="$4"
rm -f "$flag"
"$bin" --config "$cfg" >>"$log" 2>&1 &
client=$!
trap 'kill -TERM "$client" 2>/dev/null' TERM INT
while kill -0 "$client" 2>/dev/null; do
    if [ -e "$flag" ]; then
        kill -TERM "$client" 2>/dev/null
        break
    fi
    sleep 1
done
wait "$client"
status=$?
rm -f "$flag"
exit $status
"#;
        std::fs::write(&wrapper, WRAPPER_SH)
            .with_context(|| format!("write {}", wrapper.display()))?;
        let _ = std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700));

        // Truncate the log and clear any stale stop flag before starting.
        let _ = std::fs::write(&log_file, b"");
        let _ = std::fs::remove_file(&stop_flag);

        #[cfg(target_os = "linux")]
        let cmd = {
            let mut cmd = Command::new("pkexec");
            cmd.arg("sh")
                .arg(&wrapper)
                .arg(binary_path)
                .arg(&self.config_path)
                .arg(&log_file)
                .arg(&stop_flag);
            cmd
        };

        #[cfg(target_os = "macos")]
        let cmd = {
            fn q(p: &std::path::Path) -> String {
                // single-quote for sh, escape internal single quotes
                format!("'{}'", p.display().to_string().replace('\'', "'\\''"))
            }
            let sh_cmd = format!(
                "sh {} {} {} {} {}",
                q(&wrapper),
                q(binary_path),
                q(&self.config_path),
                q(&log_file),
                q(&stop_flag)
            );
            let escaped = sh_cmd.replace('\\', "\\\\").replace('"', "\\\"");
            let script = format!(
                "do shell script \"{}\" with administrator privileges",
                escaped
            );
            let mut cmd = Command::new("osascript");
            cmd.arg("-e").arg(script);
            cmd
        };

        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let cmd: Command =
            anyhow::bail!("elevated client launch is not supported on this platform");

        Ok((cmd, Some(stop_flag), Some(log_file)))
    }

    pub async fn disconnect(&self, app: AppHandle) -> Result<()> {
        self.push_log(&app, "=== Отключение ===".into(), "info");
        let prev_profile = {
            let s = self.state.lock().unwrap();
            (s.profile_id.clone(), s.profile_name.clone())
        };
        self.disconnect_internal().await?;
        let new = StatePayload {
            state: ConnectionState::Disconnected,
            profile_id: prev_profile.0,
            profile_name: prev_profile.1,
            message: Some("Отключено".into()),
            started_at: None,
        };
        self.set_state(&app, new);
        Ok(())
    }

    async fn disconnect_internal(&self) -> Result<()> {
        let ctl = self.session.lock().unwrap().take();
        let stop_flag = ctl.as_ref().and_then(|c| c.stop_flag.clone());
        if let Some(ctl) = &ctl {
            ctl.alive.store(false, Ordering::Relaxed);
        }

        let mut slot = self.child.lock().await;
        if let Some(mut child) = slot.take() {
            // Elevated Unix session: we cannot signal the root-owned client
            // directly. Create the stop-flag file — the root wrapper script
            // notices it within ~1s, TERMs the client and exits.
            if let Some(flag) = &stop_flag {
                let _ = std::fs::write(flag, b"stop");
                let _ = tokio::time::timeout(std::time::Duration::from_secs(6), child.wait()).await;
            }

            #[cfg(windows)]
            {
                use windows::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
                if let Some(pid) = child.id() {
                    unsafe {
                        let _ = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid);
                    }
                    // small grace period
                    let _ =
                        tokio::time::timeout(std::time::Duration::from_millis(1500), child.wait())
                            .await;
                }
            }
            // Force kill if still alive (no-op if already exited).
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        if let Some(flag) = &stop_flag {
            let _ = std::fs::remove_file(flag);
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_pipe_reader(
    pipe: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    app: AppHandle,
    state: Arc<Mutex<StatePayload>>,
    log_buffer: Arc<Mutex<VecDeque<LogPayload>>>,
    profile_id: String,
    profile_name: String,
    default_level: &'static str,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(pipe).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            handle_log_line(
                &app,
                &state,
                &log_buffer,
                &profile_id,
                &profile_name,
                &line,
                default_level,
            );
        }
    });
}

fn handle_log_line(
    app: &AppHandle,
    state: &Arc<Mutex<StatePayload>>,
    log_buffer: &Arc<Mutex<VecDeque<LogPayload>>>,
    profile_id: &str,
    profile_name: &str,
    line: &str,
    default_level: &str,
) {
    if line.trim().is_empty() {
        return;
    }
    let lower = line.to_ascii_lowercase();
    let level = if lower.contains("error")
        || lower.contains(" err ")
        || lower.contains("failed")
        || lower.contains("fatal")
    {
        "error"
    } else if lower.contains("warn") {
        "warn"
    } else {
        default_level
    };

    {
        let payload = LogPayload {
            line: line.to_string(),
            level: level.to_string(),
        };
        let mut buf = log_buffer.lock().unwrap();
        if buf.len() >= MAX_LOG_LINES {
            buf.pop_front();
        }
        buf.push_back(payload.clone());
        let _ = app.emit("vpn://log", &payload);
    }

    let mut transition: Option<StatePayload> = None;
    {
        let cur = state.lock().unwrap().clone();
        // Connection-up heuristics: "connected", "established", "tunnel ready", "session opened"
        let up_markers = [
            "connected to",
            "tunnel established",
            "tunnel ready",
            "session established",
            "successfully connected",
            "vpn active",
        ];
        let err_markers = [
            "authentication failed",
            "connection failed",
            "unable to connect",
            "fatal",
            "could not start",
        ];

        if cur.state == ConnectionState::Connecting && up_markers.iter().any(|m| lower.contains(m))
        {
            transition = Some(StatePayload {
                state: ConnectionState::Connected,
                profile_id: Some(profile_id.into()),
                profile_name: Some(profile_name.into()),
                message: Some("Соединение установлено".into()),
                started_at: Some(chrono::Utc::now().timestamp()),
            });
        } else if err_markers.iter().any(|m| lower.contains(m)) {
            transition = Some(StatePayload {
                state: ConnectionState::Error,
                profile_id: Some(profile_id.into()),
                profile_name: Some(profile_name.into()),
                message: Some(line.to_string()),
                started_at: None,
            });
        }
    }
    if let Some(new) = transition {
        {
            let mut s = state.lock().unwrap();
            *s = new.clone();
        }
        let _ = app.emit("vpn://state", &new);
    }
}
