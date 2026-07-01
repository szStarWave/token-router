pub fn run_apply(_target: &str, _package: &str, _temp_dir: &str) -> Result<(), String> {
    Err("OTA apply is only supported on Windows".into())
}

pub fn spawn_ota_apply_detached(
    _self_exe: &str,
    _target_abs: &str,
    _package_abs: &str,
    _temp_dir: &str,
) -> Result<(), String> {
    Err("OTA apply is only supported on Windows".into())
}
