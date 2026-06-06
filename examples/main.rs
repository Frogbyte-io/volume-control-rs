use volume_control_rs::AudioController;

fn main() {
    let mut controller = AudioController::init(None);
    controller.get_sessions();
    controller.get_default_audio_endpoint_volume_control();
    controller.get_all_process_sessions();
    controller.get_all_audio_devices();

    println!("All sessions:");
    for name in controller.get_all_session_names() {
        println!("  - {}", name);
    }

    println!("\nAudio devices:");
    for name in controller.get_all_device_names() {
        println!("  - {}", name);
    }

    println!("\nOutput devices:");
    for name in controller.get_output_device_names() {
        println!("  - {}", name);
    }

    println!("\nInput devices:");
    for name in controller.get_input_device_names() {
        println!("  - {}", name);
    }

    let master_sessions = controller.get_sessions_by_name("master".to_string());
    if let Some(session) = master_sessions.first() {
        println!("\nMaster volume: {:.2}", session.get_volume());
    } else {
        println!("\nNo master session found");
    }
}
