#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(not(any(windows, target_os = "macos")))]
pub mod stub;

#[cfg(windows)]
pub use windows::{run_apply, spawn_ota_apply_detached};

#[cfg(target_os = "macos")]
pub use macos::{app_bundle_path, run_apply, spawn_ota_apply_detached};

#[cfg(not(any(windows, target_os = "macos")))]
pub use stub::{run_apply, spawn_ota_apply_detached};
