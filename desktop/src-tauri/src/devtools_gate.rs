#[cfg(windows)]
pub fn apply_dev_access(platform: &tauri::webview::PlatformWebview, enabled: bool) {
    unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
        use windows_core::Interface;

        let controller = platform.controller();
        let webview = match controller.CoreWebView2() {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[devtools-gate] CoreWebView2: {e}");
                return;
            }
        };

        let settings = match webview.Settings() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[devtools-gate] Settings: {e}");
                return;
            }
        };

        if let Err(e) = settings.SetAreDefaultContextMenusEnabled(enabled) {
            eprintln!("[devtools-gate] SetAreDefaultContextMenusEnabled: {e}");
        }
        if let Err(e) = settings.SetAreDevToolsEnabled(enabled) {
            eprintln!("[devtools-gate] SetAreDevToolsEnabled: {e}");
        }
        if let Ok(settings3) = settings.cast::<ICoreWebView2Settings3>() {
            if let Err(e) = settings3.SetAreBrowserAcceleratorKeysEnabled(enabled) {
                eprintln!("[devtools-gate] SetAreBrowserAcceleratorKeysEnabled: {e}");
            }
        }
    }
}

#[cfg(not(windows))]
pub fn apply_dev_access(_platform: &tauri::webview::PlatformWebview, _enabled: bool) {}
