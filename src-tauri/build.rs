fn main() {
    #[cfg(windows)]
    {
        let manifest = std::fs::read_to_string("app.manifest").expect("read app.manifest");
        let attrs =
            tauri_build::WindowsAttributes::new_without_app_manifest().app_manifest(manifest);
        tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(attrs))
            .expect("tauri_build failed");
    }
    #[cfg(not(windows))]
    tauri_build::build();
}
