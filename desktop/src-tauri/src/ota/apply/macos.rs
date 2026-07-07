use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

pub fn run_apply(target: &str, package: &str, temp_dir: &str) -> Result<(), String> {
    let target_app = fs::canonicalize(target).map_err(|e| format!("resolve target app: {e}"))?;
    let package_abs = fs::canonicalize(package).map_err(|e| format!("resolve package: {e}"))?;

    let self_exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let self_exe = fs::canonicalize(&self_exe).map_err(|e| e.to_string())?;

    if let Some(running_bundle) = app_bundle_path(&self_exe) {
        let running_bundle = fs::canonicalize(&running_bundle).unwrap_or(running_bundle);
        if paths_equal(&running_bundle, &target_app) {
            thread::sleep(Duration::from_millis(800));
            return apply_from_dmg(&target_app, &package_abs);
        }
    }

    spawn_ota_apply_detached(
        &self_exe.to_string_lossy(),
        &target_app.to_string_lossy(),
        &package_abs.to_string_lossy(),
        temp_dir,
    )
}

pub fn spawn_ota_apply_detached(
    self_exe: &str,
    target_abs: &str,
    package_abs: &str,
    temp_dir: &str,
) -> Result<(), String> {
    fs::create_dir_all(temp_dir).map_err(|e| format!("temp dir: {e}"))?;

    Command::new(self_exe)
        .args([
            "ota",
            "apply",
            "--target",
            target_abs,
            "--package",
            package_abs,
            "--temp-dir",
            temp_dir,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn ota apply helper: {e}"))?;
    Ok(())
}

fn apply_from_dmg(target_app: &Path, package: &Path) -> Result<(), String> {
    wait_for_other_instances(target_app, std::process::id())?;

    let (mount_point, mount_device) = mount_dmg(package)?;
    let result = (|| {
        let source_app = find_app_bundle(&mount_point).ok_or_else(|| {
            format!(
                "no .app bundle found in mounted volume: {}",
                mount_point.display()
            )
        })?;
        replace_app_bundle(target_app, &source_app)?;
        reopen_app(target_app)
    })();

    let _ = unmount_dmg(&mount_device);
    result
}

fn wait_for_other_instances(target_app: &Path, skip_pid: u32) -> Result<(), String> {
    let target = fs::canonicalize(target_app).unwrap_or_else(|_| target_app.to_path_buf());
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if !other_instances_running(&target, skip_pid)? {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    Err(format!(
        "timed out waiting for other instances of {} to exit",
        target.display()
    ))
}

fn other_instances_running(target_app: &Path, skip_pid: u32) -> Result<bool, String> {
    let output = Command::new("pgrep")
        .arg("-f")
        .arg(target_app.to_string_lossy().as_ref())
        .output()
        .map_err(|e| format!("pgrep failed: {e}"))?;
    if !output.status.success() {
        return Ok(false);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let running = stdout
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .any(|pid| pid != skip_pid);
    Ok(running)
}

fn mount_dmg(package: &Path) -> Result<(PathBuf, String), String> {
    let output = Command::new("hdiutil")
        .args(["attach", "-nobrowse", "-readonly"])
        .arg(package)
        .output()
        .map_err(|e| format!("hdiutil attach failed: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("hdiutil attach failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut mount_point = None;
    let mut mount_device = None;
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 && parts[1] == "Apple_HFS" {
            mount_device = Some(parts[0].to_string());
            mount_point = Some(PathBuf::from(parts[2..].join(" ")));
            break;
        }
        if parts.len() >= 2 && parts[0].starts_with("/dev/") {
            mount_device = Some(parts[0].to_string());
            mount_point = Some(PathBuf::from(parts.last().copied().unwrap_or("")));
        }
    }

    let mount_point = mount_point.filter(|p| p.is_dir()).ok_or_else(|| {
        format!("failed to parse hdiutil attach output: {stdout}")
    })?;
    let mount_device = mount_device.ok_or_else(|| "failed to parse mount device".to_string())?;
    Ok((mount_point, mount_device))
}

fn unmount_dmg(device: &str) -> Result<(), String> {
    let output = Command::new("hdiutil")
        .args(["detach", device])
        .output()
        .map_err(|e| format!("hdiutil detach failed: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("hdiutil detach failed: {stderr}"))
    }
}

fn find_app_bundle(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("app") {
            return Some(path);
        }
    }
    None
}

fn replace_app_bundle(dest: &Path, src: &Path) -> Result<(), String> {
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|e| format!("remove old app bundle: {e}"))?;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create parent dir: {e}"))?;
    }

    let status = Command::new("ditto")
        .arg(src)
        .arg(dest)
        .status()
        .map_err(|e| format!("ditto failed: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("ditto exited with {status}"))
    }
}

fn reopen_app(app: &Path) -> Result<(), String> {
    Command::new("open")
        .arg("-a")
        .arg(app)
        .spawn()
        .map_err(|e| format!("reopen app failed: {e}"))?;
    Ok(())
}

pub fn app_bundle_path(exe: &Path) -> Option<PathBuf> {
    let macos_dir = exe.parent()?; // MacOS
    if macos_dir.file_name()?.to_str()? != "MacOS" {
        return None;
    }
    let contents = macos_dir.parent()?;
    if contents.file_name()?.to_str()? != "Contents" {
        return None;
    }
    let app = contents.parent()?;
    if app.extension().and_then(|ext| ext.to_str()) == Some("app") {
        Some(app.to_path_buf())
    } else {
        None
    }
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let a = fs::canonicalize(a).ok();
    let b = fs::canonicalize(b).ok();
    match (a, b) {
        (Some(aa), Some(bb)) => aa == bb,
        _ => false,
    }
}
