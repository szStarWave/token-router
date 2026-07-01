#[cfg(windows)]
pub mod windows;

#[cfg(not(windows))]
pub mod stub;

#[cfg(windows)]
pub use windows::{run_apply, spawn_ota_apply_detached};

#[cfg(not(windows))]
pub use stub::{run_apply, spawn_ota_apply_detached};
