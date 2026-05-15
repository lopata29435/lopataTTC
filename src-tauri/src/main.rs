// No console window — both debug and release use the GUI subsystem on Windows.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    trusttunnel_gui_lib::run();
}
