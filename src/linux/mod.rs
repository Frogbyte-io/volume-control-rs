use std::sync::{Arc, Condvar, Mutex};
use std::process::exit;

use libpulse_binding as pulse;
use pulse::context::{Context, FlagSet as ContextFlagSet, State as ContextState};
use pulse::mainloop::threaded::Mainloop;
use pulse::callbacks::ListResult;
use pulse::volume::Volume;
use pulse::proplist::properties as props;

use log::error;

use crate::{CoinitMode, Session};

mod session;
use session::{LinuxApplicationSession, LinuxDeviceSession};

pub struct AudioController {
    mainloop: Arc<Mutex<Mainloop>>,
    context: Arc<Mutex<Context>>,
    sessions: Vec<Box<dyn Session>>,
    default_output_index: Option<u32>,
    default_input_index: Option<u32>,
}

type DoneSignal = Arc<(Mutex<bool>, Condvar)>;

fn make_signal() -> DoneSignal {
    Arc::new((Mutex::new(false), Condvar::new()))
}

fn wait_for_signal(signal: &DoneSignal) {
    let (lock, cvar) = &**signal;
    let mut done = lock.lock().unwrap();
    while !*done {
        done = cvar.wait(done).unwrap();
    }
}

fn fire_signal(signal: &DoneSignal) {
    let (lock, cvar) = &**signal;
    *lock.lock().unwrap() = true;
    cvar.notify_one();
}

/// Acquire the PulseAudio threaded-mainloop lock. Returns the std mutex guard
/// that owns the `&mut Mainloop`; pass it to [`pa_unlock`] to release the PA
/// lock (and then the std mutex) once the introspect operation is submitted.
fn pa_lock(mainloop: &Arc<Mutex<Mainloop>>) -> std::sync::MutexGuard<'_, Mainloop> {
    let mut guard = mainloop.lock().unwrap();
    guard.lock();
    guard
}

fn pa_unlock(mut guard: std::sync::MutexGuard<'_, Mainloop>) {
    guard.unlock();
}

fn avg_volume_to_linear(cv: &pulse::volume::ChannelVolumes) -> f32 {
    cv.avg().0 as f64 as f32 / Volume::NORMAL.0 as f32
}

fn get_process_name_from_proc(pid_str: &str) -> Option<String> {
    let pid: u32 = pid_str.trim().parse().ok()?;
    std::fs::read_to_string(format!("/proc/{}/comm", pid))
        .ok()
        .map(|s| s.trim().to_string())
}

impl AudioController {
    pub fn init(_coinit_mode: Option<CoinitMode>) -> Self {
        let mut mainloop = Mainloop::new().unwrap_or_else(|| {
            eprintln!("ERROR: Failed to create PulseAudio threaded mainloop");
            error!("ERROR: Failed to create PulseAudio threaded mainloop");
            exit(1);
        });

        let mut context = Context::new(&mainloop, "volume-control-rs").unwrap_or_else(|| {
            eprintln!("ERROR: Failed to create PulseAudio context");
            error!("ERROR: Failed to create PulseAudio context");
            exit(1);
        });

        context
            .connect(None, ContextFlagSet::NOFLAGS, None)
            .unwrap_or_else(|err| {
                eprintln!("ERROR: Failed to connect to PulseAudio: {:?}", err);
                error!("ERROR: Failed to connect to PulseAudio: {:?}", err);
                exit(1);
            });

        mainloop.start().unwrap_or_else(|err| {
            eprintln!("ERROR: Failed to start PulseAudio mainloop: {:?}", err);
            error!("ERROR: Failed to start PulseAudio mainloop: {:?}", err);
            exit(1);
        });

        // Wait until the context is ready, polling its state under the mainloop lock.
        loop {
            mainloop.lock();
            let state = context.get_state();
            mainloop.unlock();
            match state {
                ContextState::Ready => break,
                ContextState::Failed | ContextState::Terminated => {
                    eprintln!("ERROR: PulseAudio context failed to connect");
                    error!("ERROR: PulseAudio context failed to connect");
                    exit(1);
                }
                _ => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        }

        Self {
            mainloop: Arc::new(Mutex::new(mainloop)),
            context: Arc::new(Mutex::new(context)),
            sessions: vec![],
            default_output_index: None,
            default_input_index: None,
        }
    }

    /// No-op on Linux — mainloop is already running after init.
    pub fn get_sessions(&mut self) {}

    pub fn get_default_audio_endpoint_volume_control(&mut self) {
        // --- Default output sink ---
        {
            let signal = make_signal();
            let signal_cb = signal.clone();

            let mainloop = self.mainloop.clone();
            let context = self.context.clone();

            struct SinkResult {
                index: u32,
                name: String,
                channels: u8,
                volume: f32,
                muted: bool,
            }
            let result: Arc<Mutex<Option<SinkResult>>> = Arc::new(Mutex::new(None));
            let result_cb = result.clone();

            let pa_guard = pa_lock(&mainloop);
            context.lock().unwrap().introspect().get_sink_info_by_name(
                "@DEFAULT_SINK@",
                move |list| match list {
                    ListResult::Item(sink) => {
                        let name = sink
                            .description
                            .as_deref()
                            .unwrap_or("Default Output")
                            .to_string();
                        let channels = sink.channel_map.len() as u8;
                        let vol = avg_volume_to_linear(&sink.volume);
                        *result_cb.lock().unwrap() = Some(SinkResult {
                            index: sink.index,
                            name,
                            channels,
                            volume: vol,
                            muted: sink.mute,
                        });
                    }
                    ListResult::End | ListResult::Error => {
                        fire_signal(&signal_cb);
                    }
                },
            );
            pa_unlock(pa_guard);
            wait_for_signal(&signal);

            let taken = result.lock().unwrap().take();
            if let Some(r) = taken {
                self.default_output_index = Some(r.index);

                self.sessions.push(Box::new(LinuxDeviceSession::new(
                    r.index,
                    true,
                    format!("output: {} (default_output)", r.name),
                    r.channels,
                    r.volume,
                    r.muted,
                    self.mainloop.clone(),
                    self.context.clone(),
                )));

                // "master" alias for backwards compat
                self.sessions.push(Box::new(LinuxDeviceSession::new(
                    r.index,
                    true,
                    "master".to_string(),
                    r.channels,
                    r.volume,
                    r.muted,
                    self.mainloop.clone(),
                    self.context.clone(),
                )));
            } else {
                eprintln!("WARNING: Could not get default output sink info");
            }
        }

        // --- Default input source ---
        {
            let signal = make_signal();
            let signal_cb = signal.clone();

            struct SourceResult {
                index: u32,
                name: String,
                channels: u8,
                volume: f32,
                muted: bool,
            }
            let result: Arc<Mutex<Option<SourceResult>>> = Arc::new(Mutex::new(None));
            let result_cb = result.clone();

            let pa_guard = pa_lock(&self.mainloop);
            self.context.lock().unwrap().introspect().get_source_info_by_name(
                "@DEFAULT_SOURCE@",
                move |list| match list {
                    ListResult::Item(source) => {
                        let name = source
                            .description
                            .as_deref()
                            .unwrap_or("Default Input")
                            .to_string();
                        let channels = source.channel_map.len() as u8;
                        let vol = avg_volume_to_linear(&source.volume);
                        *result_cb.lock().unwrap() = Some(SourceResult {
                            index: source.index,
                            name,
                            channels,
                            volume: vol,
                            muted: source.mute,
                        });
                    }
                    ListResult::End | ListResult::Error => {
                        fire_signal(&signal_cb);
                    }
                },
            );
            pa_unlock(pa_guard);
            wait_for_signal(&signal);

            let taken = result.lock().unwrap().take();
            if let Some(r) = taken {
                self.default_input_index = Some(r.index);

                self.sessions.push(Box::new(LinuxDeviceSession::new(
                    r.index,
                    false,
                    format!("input: {} (default_input)", r.name),
                    r.channels,
                    r.volume,
                    r.muted,
                    self.mainloop.clone(),
                    self.context.clone(),
                )));

                // "mic" alias for backwards compat
                self.sessions.push(Box::new(LinuxDeviceSession::new(
                    r.index,
                    false,
                    "mic".to_string(),
                    r.channels,
                    r.volume,
                    r.muted,
                    self.mainloop.clone(),
                    self.context.clone(),
                )));
            } else {
                eprintln!("WARNING: Could not get default input source info");
            }
        }
    }

    pub fn get_all_process_sessions(&mut self) {
        struct SessionData {
            index: u32,
            name: String,
            channels: u8,
            volume: f32,
            muted: bool,
        }

        let collected: Arc<Mutex<Vec<SessionData>>> = Arc::new(Mutex::new(vec![]));
        let collected_cb = collected.clone();
        let signal = make_signal();
        let signal_cb = signal.clone();

        let pa_guard = pa_lock(&self.mainloop);
        self.context.lock().unwrap().introspect().get_sink_input_info_list(move |list| {
            match list {
                ListResult::Item(sink_input) => {
                    // Skip inputs with no client (e.g. loopback/system streams)
                    if sink_input.client.is_none() {
                        return;
                    }

                    let proplist = &sink_input.proplist;

                    // Prefer application.name, fall back to /proc/{pid}/comm
                    let name = proplist
                        .get_str(props::APPLICATION_NAME)
                        .filter(|s| !s.is_empty())
                        .or_else(|| {
                            proplist
                                .get_str(props::APPLICATION_PROCESS_ID)
                                .as_deref()
                                .and_then(get_process_name_from_proc)
                        })
                        .unwrap_or_else(|| "Unknown".to_string());

                    let channels = sink_input.channel_map.len() as u8;
                    let vol = avg_volume_to_linear(&sink_input.volume);

                    collected_cb.lock().unwrap().push(SessionData {
                        index: sink_input.index,
                        name,
                        channels,
                        volume: vol,
                        muted: sink_input.mute,
                    });
                }
                ListResult::End | ListResult::Error => {
                    fire_signal(&signal_cb);
                }
            }
        });
        pa_unlock(pa_guard);
        wait_for_signal(&signal);

        for data in collected.lock().unwrap().drain(..) {
            self.sessions.push(Box::new(LinuxApplicationSession::new(
                data.index,
                data.name,
                data.channels,
                data.volume,
                data.muted,
                self.mainloop.clone(),
                self.context.clone(),
            )));
        }
    }

    pub fn get_all_audio_devices(&mut self) {
        // --- Non-default output sinks ---
        {
            struct SinkData {
                index: u32,
                name: String,
                channels: u8,
                volume: f32,
                muted: bool,
            }

            let collected: Arc<Mutex<Vec<SinkData>>> = Arc::new(Mutex::new(vec![]));
            let collected_cb = collected.clone();
            let signal = make_signal();
            let signal_cb = signal.clone();
            let default_index = self.default_output_index;

            let pa_guard = pa_lock(&self.mainloop);
            self.context.lock().unwrap().introspect().get_sink_info_list(move |list| {
                match list {
                    ListResult::Item(sink) => {
                        if Some(sink.index) == default_index {
                            return;
                        }
                        let name = sink
                            .description
                            .as_deref()
                            .unwrap_or("Unknown Output")
                            .to_string();
                        let channels = sink.channel_map.len() as u8;
                        let vol = avg_volume_to_linear(&sink.volume);
                        collected_cb.lock().unwrap().push(SinkData {
                            index: sink.index,
                            name,
                            channels,
                            volume: vol,
                            muted: sink.mute,
                        });
                    }
                    ListResult::End | ListResult::Error => {
                        fire_signal(&signal_cb);
                    }
                }
            });
            pa_unlock(pa_guard);
            wait_for_signal(&signal);

            for data in collected.lock().unwrap().drain(..) {
                self.sessions.push(Box::new(LinuxDeviceSession::new(
                    data.index,
                    true,
                    format!("output: {}", data.name),
                    data.channels,
                    data.volume,
                    data.muted,
                    self.mainloop.clone(),
                    self.context.clone(),
                )));
            }
        }

        // --- Non-default input sources (skip monitors) ---
        {
            struct SourceData {
                index: u32,
                name: String,
                channels: u8,
                volume: f32,
                muted: bool,
            }

            let collected: Arc<Mutex<Vec<SourceData>>> = Arc::new(Mutex::new(vec![]));
            let collected_cb = collected.clone();
            let signal = make_signal();
            let signal_cb = signal.clone();
            let default_index = self.default_input_index;

            let pa_guard = pa_lock(&self.mainloop);
            self.context.lock().unwrap().introspect().get_source_info_list(move |list| {
                match list {
                    ListResult::Item(source) => {
                        if Some(source.index) == default_index {
                            return;
                        }
                        // Skip monitor sources (passthrough of sink output)
                        let source_name = source.name.as_deref().unwrap_or("");
                        if source_name.ends_with(".monitor") {
                            return;
                        }
                        let description = source
                            .description
                            .as_deref()
                            .unwrap_or("Unknown Input")
                            .to_string();
                        let channels = source.channel_map.len() as u8;
                        let vol = avg_volume_to_linear(&source.volume);
                        collected_cb.lock().unwrap().push(SourceData {
                            index: source.index,
                            name: description,
                            channels,
                            volume: vol,
                            muted: source.mute,
                        });
                    }
                    ListResult::End | ListResult::Error => {
                        fire_signal(&signal_cb);
                    }
                }
            });
            pa_unlock(pa_guard);
            wait_for_signal(&signal);

            for data in collected.lock().unwrap().drain(..) {
                self.sessions.push(Box::new(LinuxDeviceSession::new(
                    data.index,
                    false,
                    format!("input: {}", data.name),
                    data.channels,
                    data.volume,
                    data.muted,
                    self.mainloop.clone(),
                    self.context.clone(),
                )));
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

impl Drop for AudioController {
    fn drop(&mut self) {
        if let Ok(mut mainloop) = self.mainloop.lock() {
            mainloop.stop();
        }
    }
}
