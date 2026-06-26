#[cfg(windows)]
use std::collections::HashSet;
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
static REGISTERED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[cfg(windows)]
fn is_wechat_callback_url(url: &str) -> bool {
    url.contains("/auth/third/callback") && !url.contains("channel=")
}

#[cfg(windows)]
pub fn register_wechat_iframe_interceptor(
    platform: tauri::webview::PlatformWebview,
    app: AppHandle,
    label: String,
) {
    let registered = REGISTERED.get_or_init(|| Mutex::new(HashSet::new()));
    if registered.lock().map(|s| s.contains(&label)).unwrap_or(false) {
        return;
    }
    registered.lock().map(|mut s| s.insert(label.clone())).ok();

    unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        use webview2_com::WebResourceRequestedEventHandler;
        use windows_core::{HSTRING, Interface};

        let controller = platform.controller();
        let env = platform.environment();
        let webview = match controller.CoreWebView2() {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[wechat-callback] CoreWebView2: {e}");
                return;
            }
        };

        let filter = HSTRING::from("https://server.flowyaipc.cn/*");
        if let Ok(webview_22) = webview.cast::<ICoreWebView2_22>() {
            let _ = webview_22.AddWebResourceRequestedFilterWithRequestSourceKinds(
                &filter,
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
            );
        } else {
            let _ = webview
                .AddWebResourceRequestedFilter(&filter, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL);
        }

        let mut token: i64 = 0;
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
            let args = match args {
                Some(a) => a,
                None => return Ok(()),
            };

            let request = match args.Request() {
                Ok(r) => r,
                Err(_) => return Ok(()),
            };

            let uri = match uri_from_request(&request) {
                Some(u) => u,
                None => return Ok(()),
            };

            if !is_wechat_callback_url(&uri) {
                return Ok(());
            }

            let _ = app.emit_to(&label, "wechat-login-callback", uri.clone());

            let status = HSTRING::from("Not Found");
            let headers = HSTRING::from("");
            match env.CreateWebResourceResponse(None, 404, &status, &headers) {
                Ok(response) => {
                    let _ = args.SetResponse(&response);
                }
                Err(e) => eprintln!("[wechat-callback] CreateWebResourceResponse: {e}"),
            }

            Ok(())
        }));

        if let Err(e) = webview.add_WebResourceRequested(&handler, &mut token) {
            eprintln!("[wechat-callback] add_WebResourceRequested: {e}");
        }
    }
}

#[cfg(windows)]
unsafe fn uri_from_request(
    request: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2WebResourceRequest,
) -> Option<String> {
    use windows_core::PWSTR;

    let mut pwstr = PWSTR::null();
    request.Uri(std::ptr::addr_of_mut!(pwstr)).ok()?;
    if pwstr.is_null() {
        return None;
    }
    pwstr.to_string().ok()
}

#[cfg(not(windows))]
pub fn register_wechat_iframe_interceptor(
    _platform: tauri::webview::PlatformWebview,
    _app: tauri::AppHandle,
    _label: String,
) {
}
