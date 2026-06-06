#![cfg(target_os = "windows")]

//! Demonstrates reading the current master volume level on Windows.
//! Volume change callbacks (IAudioEndpointVolumeCallback) require enabling
//! the "windows/implement" feature; this example shows polling instead.

use volume_control_rs::AudioController;

fn main() {
    let mut controller = AudioController::init(None);
    controller.get_sessions();
    controller.get_default_audio_endpoint_volume_control();

    let session = controller
        .get_session_by_name("master".to_string())
        .expect("no master session found");

    println!("Current master volume: {:.2}", session.get_volume());
    println!("Muted: {}", session.get_mute());
}
