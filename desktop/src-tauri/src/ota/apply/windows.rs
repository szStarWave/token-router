use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOW;

const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const DETACHED_PROCESS: u32 = 0x0000_0008;
/// Tauri NSIS: silent in-place update and restart the app when finished.
const NSIS_SILENT_UPDATE_ARGS: &[&str] = &["/S", "/UPDATE", "/R"];

pub fn run_apply(target: &str, package: &str, temp_dir: &str) -> Result<(), String> {
    let target_abs = fs::canonicalize(target).map_err(|e| format!("resolve target: {e}"))?;
    let package_abs = fs::canonicalize(package).map_err(|e| format!("resolve package: {e}"))?;
    let temp_dir = PathBuf::from(temp_dir);

    let self_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let self_exe = fs::canonicalize(&self_exe).map_err(|e| e.to_string())?;

    if paths_equal(&self_exe, &target_abs) {
        return spawn_ota_apply_detached(
            &self_exe.to_string_lossy(),
            &target_abs.to_string_lossy(),
            &package_abs.to_string_lossy(),
            &temp_dir.to_string_lossy(),
        );
    }

    run_nsis_setup(&package_abs, &target_abs)
}

pub fn spawn_ota_apply_detached(
    self_exe: &str,
    target_abs: &str,
    package_abs: &str,
    temp_dir: &str,
) -> Result<(), String> {
    let self_path = PathBuf::from(self_exe);
    let temp_dir = PathBuf::from(temp_dir);
    fs::create_dir_all(&temp_dir).map_err(|e| format!("temp dir: {e}"))?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dst = temp_dir.join(format!("token_router_ota_apply_{}_{ts}.exe", std::process::id()));

    if let Err(e) = copy_executable_file(&self_path, &dst) {
        return try_elevated_spawn_message(self_exe, target_abs, package_abs, temp_dir.to_str().unwrap_or(""), &e);
    }

    let status = Command::new(&dst)
        .args([
            "ota",
            "apply",
            "--target",
            target_abs,
            "--package",
            package_abs,
            "--temp-dir",
            temp_dir.to_str().unwrap_or(""),
        ])
        .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
        .spawn();

    match status {
        Ok(_) => Ok(()),
        Err(e) => try_elevated_spawn(self_exe, target_abs, package_abs, temp_dir.to_str().unwrap_or(""), &e),
    }
}

fn run_nsis_setup(setup_abs: &Path, target_abs: &Path) -> Result<(), String> {
    if !setup_abs.is_file() {
        return Err(format!("setup package missing: {}", setup_abs.display()));
    }

    if target_abs.is_file() {
        terminate_processes_with_image_path(target_abs, std::process::id())?;
        std::thread::sleep(Duration::from_millis(500));
    }

    match run_nsis_setup_process(setup_abs) {
        Ok(()) => Ok(()),
        Err(e) if is_access_denied(&e) => {
            try_elevated_setup(setup_abs, &e)
        }
        Err(e) => Err(e),
    }
}

fn run_nsis_setup_process(setup_abs: &Path) -> Result<(), String> {
    let status = Command::new(setup_abs)
        .args(NSIS_SILENT_UPDATE_ARGS)
        .status()
        .map_err(|e| format!("launch setup installer: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("setup installer exited with {status}"))
    }
}

fn nsis_setup_args() -> Vec<String> {
    NSIS_SILENT_UPDATE_ARGS.iter().map(|s| (*s).to_string()).collect()
}

fn copy_executable_file(src: &Path, dst: &Path) -> Result<(), String> {
    let mut in_file = fs::File::open(src).map_err(|e| e.to_string())?;
    let mut out_file = fs::File::create(dst).map_err(|e| e.to_string())?;
    std::io::copy(&mut in_file, &mut out_file).map_err(|e| e.to_string())?;
    out_file.sync_all().map_err(|e| e.to_string())?;
    Ok(())
}

fn terminate_processes_with_image_path(target_abs: &Path, skip_pid: u32) -> Result<(), String> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
            .map_err(|e| format!("CreateToolhelp32Snapshot: {e}"))?;
        let _guard = HandleGuard(snap);

        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        if Process32FirstW(snap, &mut entry).is_err() {
            return Ok(());
        }

        loop {
            let pid = entry.th32ProcessID;
            if pid != 0 && pid != skip_pid {
                if let Ok(img) = query_process_image_path(pid) {
                    if paths_equal(Path::new(&img), target_abs) {
                        terminate_pid(pid)?;
                    }
                }
            }
            if Process32NextW(snap, &mut entry).is_err() {
                break;
            }
        }
    }
    Ok(())
}

unsafe fn query_process_image_path(pid: u32) -> Result<String, String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|e| format!("OpenProcess: {e}"))?;
        let _guard = HandleGuard(handle);

        let mut buf = [0u16; 32768];
        let mut size = buf.len() as u32;
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        )
        .map_err(|e| format!("QueryFullProcessImageNameW: {e}"))?;
        Ok(String::from_utf16_lossy(&buf[..size as usize]))
    }
}

fn terminate_pid(pid: u32) -> Result<(), String> {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, false, pid)
            .map_err(|e| format!("OpenProcess terminate: {e}"))?;
        let _guard = HandleGuard(handle);
        TerminateProcess(handle, 1).map_err(|e| format!("TerminateProcess: {e}"))?;
    }
    Ok(())
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let a = fs::canonicalize(a).ok();
    let b = fs::canonicalize(b).ok();
    match (a, b) {
        (Some(aa), Some(bb)) => aa.to_string_lossy().eq_ignore_ascii_case(&bb.to_string_lossy()),
        _ => false,
    }
}

fn is_access_denied(err: &str) -> bool {
    err.contains("Access is denied")
        || err.contains("access denied")
        || err.to_lowercase().contains("eacces")
}

fn try_elevated_setup(setup_abs: &Path, orig: &str) -> Result<(), String> {
    shell_execute_runas(setup_abs, &nsis_setup_args())
        .map_err(|e| format!("{orig}; elevation relaunch failed: {e}"))
}

fn try_elevated_spawn_message(
    self_exe: &str,
    target_abs: &str,
    package_abs: &str,
    temp_dir: &str,
    orig: &str,
) -> Result<(), String> {
    if !is_access_denied(orig) {
        return Err(orig.to_string());
    }
    let args = vec![
        "ota".to_string(),
        "apply".to_string(),
        "--target".to_string(),
        target_abs.to_string(),
        "--package".to_string(),
        package_abs.to_string(),
        "--temp-dir".to_string(),
        temp_dir.to_string(),
    ];
    shell_execute_runas(Path::new(self_exe), &args)
        .map_err(|e| format!("{orig}; elevation relaunch failed: {e}"))
}

fn try_elevated_spawn(
    self_exe: &str,
    target_abs: &str,
    package_abs: &str,
    temp_dir: &str,
    orig: &std::io::Error,
) -> Result<(), String> {
    let orig_msg = orig.to_string();
    if !is_access_denied(&orig_msg) {
        return Err(orig_msg);
    }
    let args = vec![
        "ota".to_string(),
        "apply".to_string(),
        "--target".to_string(),
        target_abs.to_string(),
        "--package".to_string(),
        package_abs.to_string(),
        "--temp-dir".to_string(),
        temp_dir.to_string(),
    ];
    shell_execute_runas(Path::new(self_exe), &args)
        .map_err(|e| format!("{orig_msg}; elevation relaunch failed: {e}"))
}

fn shell_execute_runas(exe: &Path, args: &[String]) -> Result<(), String> {
    let verb = to_wide("runas");
    let exe_w = path_to_wide(exe);
    let params = join_args_for_shell_execute(args);
    let params_w = if params.is_empty() {
        None
    } else {
        Some(to_wide(&params))
    };

    unsafe {
        let param_ptr = params_w
            .as_ref()
            .map(|p| windows::core::PCWSTR(p.as_ptr()))
            .unwrap_or(windows::core::PCWSTR::null());
        let result = ShellExecuteW(
            None,
            windows::core::PCWSTR(verb.as_ptr()),
            windows::core::PCWSTR(exe_w.as_ptr()),
            param_ptr,
            None,
            SW_SHOW,
        );
        if result.0 as isize <= 32 {
            return Err(format!("ShellExecuteW failed: {}", result.0 as isize));
        }
    }
    Ok(())
}

fn join_args_for_shell_execute(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.contains(' ') || a.contains('"') {
                format!("\"{}\"", a.replace('"', "\\\""))
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn path_to_wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect()
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
