use self::process_api::get_process_info;
use self::session::{ApplicationSession, EndPointSession};
use windows::{
    core::ComInterface,
    Win32::{
        Media::Audio::{
            eCapture, eMultimedia, eRender,
            Endpoints::IAudioEndpointVolume,
            IAudioSessionControl, IAudioSessionControl2, IAudioSessionEnumerator,
            IAudioSessionManager2, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator,
            ISimpleAudioVolume, MMDeviceEnumerator, PKEY_AudioEndpoint_FormFactor,
            DEVICE_STATE_ACTIVE,
        },
        System::Com::{
            CoCreateInstance, CoInitializeEx, CLSCTX_ALL, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED, STGM_READ, CoTaskMemFree,
        },
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        UI::Shell::PropertiesSystem::{IPropertyStore, PropVariantToStringAlloc},
    },
};
use std::process::exit;
use log::error;

use crate::{CoinitMode, Session};

mod process_api;
mod session;

const FORM_FACTOR_SPDIF: i32 = 8;

fn get_device_friendly_name(device: &IMMDevice, fallback_name: &str) -> String {
    unsafe {
        let mut friendly_name = fallback_name.to_string();
        if let Ok(property_store) = device.OpenPropertyStore(STGM_READ) {
            if let Ok(prop_variant) = property_store.GetValue(&PKEY_Device_FriendlyName) {
                match PropVariantToStringAlloc(&prop_variant) {
                    Ok(buffer) => {
                        if !buffer.is_null() {
                            if let Ok(name) = buffer.to_string() {
                                if !name.is_empty() {
                                    friendly_name = name;
                                }
                            }
                            CoTaskMemFree(Some(buffer.as_ptr().cast()));
                        }
                    }
                    Err(_) => {}
                }
            }
        }
        friendly_name
    }
}

fn get_device_form_factor(property_store: &IPropertyStore) -> Option<i32> {
    unsafe {
        if let Ok(prop_variant) = property_store.GetValue(&PKEY_AudioEndpoint_FormFactor) {
            if let Ok(buffer) = PropVariantToStringAlloc(&prop_variant) {
                if !buffer.is_null() {
                    let form_factor = buffer
                        .to_string()
                        .ok()
                        .and_then(|val| val.parse::<i32>().ok());
                    CoTaskMemFree(Some(buffer.as_ptr().cast()));
                    return form_factor;
                }
            }
        }
        None
    }
}

pub struct AudioController {
    default_device: Option<IMMDevice>,
    default_input_device: Option<IMMDevice>,
    imm_device_enumerator: Option<IMMDeviceEnumerator>,
    sessions: Vec<Box<dyn Session>>,
    default_output_id: Option<String>,
    default_input_id: Option<String>,
}

impl AudioController {
    pub fn init(coinit_mode: Option<CoinitMode>) -> Self {
        let mut coinit = COINIT_MULTITHREADED;
        if let Some(x) = coinit_mode {
            match x {
                CoinitMode::ApartmentThreaded => { coinit = COINIT_APARTMENTTHREADED }
                CoinitMode::MultiThreaded     => { coinit = COINIT_MULTITHREADED }
            }
        }

        unsafe {
            CoInitializeEx(None, coinit).unwrap_or_else(|err| {
                eprintln!("ERROR: Couldn't initialize windows connection: {err}");
                error!("ERROR: Couldn't initialize windows connection: {}", err);
                exit(1);
            });
        }

        Self {
            default_device: None,
            default_input_device: None,
            imm_device_enumerator: None,
            sessions: vec![],
            default_output_id: None,
            default_input_id: None,
        }
    }

    pub fn get_sessions(&mut self) {
        self.imm_device_enumerator = Some(unsafe {
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER).unwrap_or_else(
                |err| {
                    eprintln!("ERROR: Couldn't get Media device enumerator: {err}");
                    error!("ERROR: Couldn't get Media device enumerator: {}", err);
                    exit(1);
                },
            )
        });
    }

    pub fn get_all_process_sessions(&mut self) {
        unsafe {
            let device_enumerator: IMMDeviceEnumerator =
                CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER)
                    .unwrap_or_else(|err| {
                        eprintln!("ERROR: Couldn't create device enumerator... {err}");
                        error!("ERROR: Couldn't create device enumerator... {}", err);
                        exit(1);
                    });

            let device_collection: IMMDeviceCollection = device_enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .unwrap_or_else(|err| {
                    eprintln!("ERROR: Couldn't enumerate audio endpoints... {err}");
                    error!("ERROR: Couldn't enumerate audio endpoints... {}", err);
                    exit(1);
                });

            let device_count = device_collection.GetCount().unwrap_or_else(|err| {
                eprintln!("ERROR: Couldn't get device count... {err}");
                error!("ERROR: Couldn't get device count... {}", err);
                exit(1);
            });

            for device_index in 0..device_count {
                let device: IMMDevice = match device_collection.Item(device_index) {
                    Ok(dev) => dev,
                    Err(err) => {
                        eprintln!("WARNING: Skipping device {} - couldn't get device: {}", device_index, err);
                        continue;
                    }
                };

                let device_id = match device.GetId() {
                    Ok(id) => match id.to_string() {
                        Ok(id_str) => id_str,
                        Err(_) => { continue; }
                    },
                    Err(_) => { continue; }
                };

                let property_store = match device.OpenPropertyStore(STGM_READ) {
                    Ok(store) => store,
                    Err(err) => {
                        let device_name = get_device_friendly_name(&device, &format!("Device {}", device_index));
                        eprintln!("ERROR: Skipping device '{}' (ID: {}) - couldn't open property store: {}", device_name, device_id, err);
                        continue;
                    }
                };

                let device_name = get_device_friendly_name(&device, &format!("Device {}", device_index));

                if let Some(form_factor) = get_device_form_factor(&property_store) {
                    if form_factor == FORM_FACTOR_SPDIF {
                        continue;
                    }
                }

                let session_manager2: IAudioSessionManager2 = match device.Activate(CLSCTX_INPROC_SERVER, None) {
                    Ok(mgr) => mgr,
                    Err(err) => {
                        eprintln!("WARNING: Skipping device '{}' - couldn't activate AudioSessionManager: {}", device_name, err);
                        continue;
                    }
                };

                let session_enumerator: IAudioSessionEnumerator = match session_manager2.GetSessionEnumerator() {
                    Ok(e) => e,
                    Err(err) => {
                        eprintln!("WARNING: Skipping device '{}' - couldn't get session enumerator: {}", device_name, err);
                        continue;
                    }
                };

                let session_count = match session_enumerator.GetCount() {
                    Ok(c) => c,
                    Err(err) => {
                        eprintln!("WARNING: Skipping device '{}' - couldn't get session count: {}", device_name, err);
                        continue;
                    }
                };

                for i in 0..session_count {
                    let normal_session_control: Option<IAudioSessionControl> = session_enumerator.GetSession(i).ok();
                    if normal_session_control.is_none() {
                        eprintln!("ERROR: Couldn't get session control for device '{}'", device_name);
                        continue;
                    }

                    let session_control: Option<IAudioSessionControl2> = normal_session_control.unwrap().cast().ok();
                    if session_control.is_none() {
                        eprintln!("ERROR: Couldn't cast to IAudioSessionControl2 for device '{}'", device_name);
                        continue;
                    }

                    let pid = session_control.as_ref().unwrap().GetProcessId().unwrap();
                    if pid == 0 {
                        continue;
                    }

                    let session_app_name = match get_process_info(pid) {
                        Ok(info) => info.process_name.clone(),
                        Err(_) => {
                            eprintln!("ERROR: Couldn't get process info for pid {} on device '{}'", pid, device_name);
                            continue;
                        }
                    };

                    let audio_control: ISimpleAudioVolume = match session_control.unwrap().cast() {
                        Ok(data) => data,
                        Err(err) => {
                            eprintln!("ERROR: Couldn't get ISimpleAudioVolume for device '{}': {}", device_name, err);
                            continue;
                        }
                    };

                    self.sessions.push(Box::new(ApplicationSession::new(audio_control, session_app_name)));
                }
            }
        }
    }

    pub fn get_default_audio_endpoint_volume_control(&mut self) {
        if self.imm_device_enumerator.is_none() {
            eprintln!("ERROR: Function called before creating enumerator");
            error!("ERROR: Function called before creating enumerator");
            return;
        }

        unsafe {
            // Default output device
            self.default_device = match self.imm_device_enumerator
                .clone()
                .unwrap()
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
            {
                Ok(device) => Some(device),
                Err(err) => {
                    eprintln!("ERROR: Couldn't get Default audio output endpoint {err}");
                    None
                }
            };

            // Default input device
            self.default_input_device = match self.imm_device_enumerator
                .clone()
                .unwrap()
                .GetDefaultAudioEndpoint(eCapture, eMultimedia)
            {
                Ok(device) => Some(device),
                Err(err) => {
                    eprintln!("ERROR: Couldn't get Default audio input endpoint {err}");
                    None
                }
            };

            if let Some(ref device) = self.default_device {
                self.default_output_id = device.GetId().ok().and_then(|id| id.to_string().ok());

                let endpoint_volume: IAudioEndpointVolume = match device.Activate(CLSCTX_ALL, None) {
                    Ok(v) => v,
                    Err(err) => {
                        eprintln!("ERROR: Couldn't activate Endpoint volume for default output: {err}");
                        return;
                    }
                };

                let friendly_name = get_device_friendly_name(device, "Default Output");
                self.sessions.push(Box::new(EndPointSession::new(
                    endpoint_volume,
                    format!("output: {} (default_output)", friendly_name),
                )));

                if let Ok(master_vol) = device.Activate(CLSCTX_ALL, None) {
                    self.sessions.push(Box::new(EndPointSession::new(master_vol, "master".to_string())));
                }
            }

            if let Some(ref device) = self.default_input_device {
                self.default_input_id = device.GetId().ok().and_then(|id| id.to_string().ok());

                let endpoint_volume: IAudioEndpointVolume = match device.Activate(CLSCTX_ALL, None) {
                    Ok(v) => v,
                    Err(err) => {
                        eprintln!("ERROR: Couldn't activate Endpoint volume for default input: {err}");
                        return;
                    }
                };

                let friendly_name = get_device_friendly_name(device, "Default Input");
                self.sessions.push(Box::new(EndPointSession::new(
                    endpoint_volume,
                    format!("input: {} (default_input)", friendly_name),
                )));

                if let Ok(mic_vol) = device.Activate(CLSCTX_ALL, None) {
                    self.sessions.push(Box::new(EndPointSession::new(mic_vol, "mic".to_string())));
                }
            }
        }
    }

    pub fn get_all_audio_devices(&mut self) {
        if self.imm_device_enumerator.is_none() {
            self.imm_device_enumerator = Some(unsafe {
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER).unwrap_or_else(
                    |err| {
                        eprintln!("ERROR: Couldn't get Media device enumerator: {err}");
                        exit(1);
                    },
                )
            });
        }

        unsafe {
            let output_collection: IMMDeviceCollection = match self.imm_device_enumerator
                .as_ref()
                .unwrap()
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            {
                Ok(col) => col,
                Err(err) => {
                    eprintln!("ERROR: Couldn't enumerate output endpoints: {err}");
                    return;
                }
            };

            let output_count = output_collection.GetCount().unwrap();
            for i in 0..output_count {
                if let Ok(device) = output_collection.Item(i) {
                    let is_default = self.default_output_id.as_ref().map_or(false, |id| {
                        device.GetId().ok().and_then(|d| d.to_string().ok()).as_ref() == Some(id)
                    });
                    if is_default { continue; }

                    let friendly_name = get_device_friendly_name(&device, "Unknown Output Device");
                    if let Ok(endpoint_volume) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                        self.sessions.push(Box::new(EndPointSession::new(
                            endpoint_volume,
                            format!("output: {}", friendly_name),
                        )));
                    }
                }
            }

            let input_collection: IMMDeviceCollection = match self.imm_device_enumerator
                .as_ref()
                .unwrap()
                .EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)
            {
                Ok(col) => col,
                Err(err) => {
                    eprintln!("ERROR: Couldn't enumerate input endpoints: {err}");
                    return;
                }
            };

            let input_count = match input_collection.GetCount() {
                Ok(c) => c,
                Err(err) => {
                    eprintln!("ERROR: Couldn't get input device count: {err}");
                    return;
                }
            };

            for i in 0..input_count {
                if let Ok(device) = input_collection.Item(i) {
                    let is_default = self.default_input_id.as_ref().map_or(false, |id| {
                        device.GetId().ok().and_then(|d| d.to_string().ok()).as_ref() == Some(id)
                    });
                    if is_default { continue; }

                    let friendly_name = get_device_friendly_name(&device, "Unknown Input Device");
                    if let Ok(endpoint_volume) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                        self.sessions.push(Box::new(EndPointSession::new(
                            endpoint_volume,
                            format!("input: {}", friendly_name),
                        )));
                    }
                }
            }
        }
    }

    pub fn get_all_session_names(&self) -> Vec<String> {
        self.sessions.iter().map(|s| s.get_name()).collect()
    }

    pub fn get_all_device_names(&self) -> Vec<String> {
        self.sessions.iter()
            .map(|s| s.get_name())
            .filter(|name| name.starts_with("input: ") || name.starts_with("output: "))
            .collect()
    }

    pub fn get_output_device_names(&self) -> Vec<String> {
        self.sessions.iter()
            .map(|s| s.get_name())
            .filter(|name| name.starts_with("output: "))
            .collect()
    }

    pub fn get_input_device_names(&self) -> Vec<String> {
        self.sessions.iter()
            .map(|s| s.get_name())
            .filter(|name| name.starts_with("input: "))
            .collect()
    }

    pub fn get_sessions_by_name(&self, name: String) -> Vec<&Box<dyn Session>> {
        self.sessions.iter().filter(|s| s.get_name() == name).collect()
    }

    pub fn get_session_by_name(&self, name: String) -> Option<&Box<dyn Session>> {
        self.sessions.iter().find(|s| s.get_name() == name)
    }
}
