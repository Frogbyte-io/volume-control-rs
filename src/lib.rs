mod session;
mod volume_monitor;

pub use session::Session;
pub use volume_monitor::VolumeMonitor;

#[derive(Debug)]
pub enum CoinitMode {
    MultiThreaded,
    ApartmentThreaded,
}

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
pub use windows::AudioController;

#[cfg(target_os = "linux")]
pub use linux::AudioController;
