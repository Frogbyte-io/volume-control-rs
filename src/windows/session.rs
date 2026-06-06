use windows::Win32::Foundation::BOOL;
use windows::core::GUID;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::ISimpleAudioVolume;

use std::process::exit;

use crate::Session;

pub struct EndPointSession {
    audio_endpoint_volume: IAudioEndpointVolume,
    name: String,
    guid: GUID,
}

impl EndPointSession {
    pub fn new(audio_endpoint_volume: IAudioEndpointVolume, name: String) -> Self {
        let guid = GUID::new().unwrap_or_else(|err| {
            eprintln!("ERROR: Couldn't generate GUID {err}");
            exit(1);
        });
        Self { audio_endpoint_volume, name, guid }
    }

    pub fn get_audio_endpoint_volume(&self) -> Option<IAudioEndpointVolume> {
        Some(self.audio_endpoint_volume.clone())
    }
}

impl Session for EndPointSession {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_volume(&self) -> f32 {
        unsafe {
            self.audio_endpoint_volume
                .GetMasterVolumeLevelScalar()
                .unwrap_or_else(|err| {
                    eprintln!("ERROR: Couldn't get volume {err}");
                    0.0
                })
        }
    }

    fn set_volume(&self, vol: f32) {
        unsafe {
            self.audio_endpoint_volume
                .SetMasterVolumeLevelScalar(vol, &self.guid)
                .unwrap_or_else(|err| {
                    eprintln!("ERROR: Couldn't set volume: {err}");
                });
        }
    }

    fn set_mute(&self, mute: bool) {
        unsafe {
            self.audio_endpoint_volume
                .SetMute(mute, &self.guid)
                .unwrap_or_else(|err| {
                    eprintln!("ERROR: Couldn't set mute: {err}");
                });
        }
    }

    fn get_mute(&self) -> bool {
        unsafe {
            self.audio_endpoint_volume
                .GetMute()
                .unwrap_or_else(|err| {
                    eprintln!("ERROR: Couldn't get mute {err}");
                    BOOL(0)
                })
                .as_bool()
        }
    }
}

pub struct ApplicationSession {
    simple_audio_volume: ISimpleAudioVolume,
    name: String,
    guid: GUID,
}

impl ApplicationSession {
    pub fn new(simple_audio_volume: ISimpleAudioVolume, name: String) -> Self {
        let guid = GUID::new().unwrap_or_else(|err| {
            eprintln!("ERROR: Couldn't generate GUID {err}");
            exit(1);
        });
        Self { simple_audio_volume, name, guid }
    }
}

// COM interface pointers are not Send by default. These are used only within
// a single Tauri command invocation (same thread), so marking Send is safe here.
unsafe impl Send for EndPointSession {}
unsafe impl Send for ApplicationSession {}

impl Session for ApplicationSession {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_volume(&self) -> f32 {
        unsafe {
            self.simple_audio_volume
                .GetMasterVolume()
                .unwrap_or_else(|err| {
                    eprintln!("ERROR: Couldn't get volume {err}");
                    0.0
                })
        }
    }

    fn set_volume(&self, vol: f32) {
        unsafe {
            self.simple_audio_volume
                .SetMasterVolume(vol, &self.guid)
                .unwrap_or_else(|err| {
                    eprintln!("ERROR: Couldn't set volume: {err}");
                });
        }
    }

    fn set_mute(&self, mute: bool) {
        unsafe {
            self.simple_audio_volume
                .SetMute(mute, &self.guid)
                .unwrap_or_else(|err| {
                    eprintln!("ERROR: Couldn't set mute: {err}");
                });
        }
    }

    fn get_mute(&self) -> bool {
        unsafe {
            self.simple_audio_volume
                .GetMute()
                .unwrap_or_else(|err| {
                    eprintln!("ERROR: Couldn't get mute {err}");
                    BOOL(0)
                })
                .as_bool()
        }
    }
}
