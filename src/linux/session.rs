use std::sync::{Arc, Condvar, Mutex};

use libpulse_binding as pulse;
use pulse::mainloop::threaded::Mainloop;
use pulse::context::Context;
use pulse::volume::{ChannelVolumes, Volume, VolumeLinear};

use crate::Session;

fn linear_to_channel_volumes(vol: f32, channels: u8) -> ChannelVolumes {
    let pa_vol = Volume::from(VolumeLinear(vol as f64));
    let mut cv = ChannelVolumes::default();
    cv.set(channels, pa_vol);
    cv
}

type DoneSignal = Arc<(Mutex<bool>, Condvar)>;

fn make_signal() -> DoneSignal {
    Arc::new((Mutex::new(false), Condvar::new()))
}

/// Build the success callback for an introspect set-operation. It fires the
/// signal so the caller can block until the change has reached the server.
fn completion_cb(signal: &DoneSignal) -> Box<dyn FnMut(bool)> {
    let signal = signal.clone();
    Box::new(move |_success| {
        let (lock, cvar) = &*signal;
        *lock.lock().unwrap() = true;
        cvar.notify_one();
    })
}

fn wait_for_signal(signal: &DoneSignal) {
    let (lock, cvar) = &**signal;
    let mut done = lock.lock().unwrap();
    while !*done {
        done = cvar.wait(done).unwrap();
    }
}

pub struct LinuxApplicationSession {
    pub index: u32,
    pub name: String,
    pub channels: u8,
    volume: Mutex<f32>,
    muted: Mutex<bool>,
    mainloop: Arc<Mutex<Mainloop>>,
    context: Arc<Mutex<Context>>,
}

impl LinuxApplicationSession {
    pub fn new(
        index: u32,
        name: String,
        channels: u8,
        volume: f32,
        muted: bool,
        mainloop: Arc<Mutex<Mainloop>>,
        context: Arc<Mutex<Context>>,
    ) -> Self {
        Self {
            index,
            name,
            channels,
            volume: Mutex::new(volume),
            muted: Mutex::new(muted),
            mainloop,
            context,
        }
    }
}

impl Session for LinuxApplicationSession {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_volume(&self) -> f32 {
        *self.volume.lock().unwrap()
    }

    fn set_volume(&self, vol: f32) {
        let clamped = vol.clamp(0.0, 1.0);
        let cv = linear_to_channel_volumes(clamped, self.channels.max(1));
        let index = self.index;
        let signal = make_signal();
        {
            let mut ml = self.mainloop.lock().unwrap();
            ml.lock();
            self.context
                .lock()
                .unwrap()
                .introspect()
                .set_sink_input_volume(index, &cv, Some(completion_cb(&signal)));
            ml.unlock();
        }
        wait_for_signal(&signal);
        *self.volume.lock().unwrap() = clamped;
    }

    fn set_mute(&self, mute: bool) {
        let index = self.index;
        let signal = make_signal();
        {
            let mut ml = self.mainloop.lock().unwrap();
            ml.lock();
            self.context
                .lock()
                .unwrap()
                .introspect()
                .set_sink_input_mute(index, mute, Some(completion_cb(&signal)));
            ml.unlock();
        }
        wait_for_signal(&signal);
        *self.muted.lock().unwrap() = mute;
    }

    fn get_mute(&self) -> bool {
        *self.muted.lock().unwrap()
    }
}

// LinuxApplicationSession is Send because all interior mutability is Mutex-guarded
// and libpulse Mainloop/Context are accessed only while holding the mainloop lock.
unsafe impl Send for LinuxApplicationSession {}

pub struct LinuxDeviceSession {
    pub index: u32,
    pub is_output: bool,
    pub name: String,
    pub channels: u8,
    volume: Mutex<f32>,
    muted: Mutex<bool>,
    mainloop: Arc<Mutex<Mainloop>>,
    context: Arc<Mutex<Context>>,
}

impl LinuxDeviceSession {
    pub fn new(
        index: u32,
        is_output: bool,
        name: String,
        channels: u8,
        volume: f32,
        muted: bool,
        mainloop: Arc<Mutex<Mainloop>>,
        context: Arc<Mutex<Context>>,
    ) -> Self {
        Self {
            index,
            is_output,
            name,
            channels,
            volume: Mutex::new(volume),
            muted: Mutex::new(muted),
            mainloop,
            context,
        }
    }
}

impl Session for LinuxDeviceSession {
    fn get_name(&self) -> String {
        self.name.clone()
    }

    fn get_volume(&self) -> f32 {
        *self.volume.lock().unwrap()
    }

    fn set_volume(&self, vol: f32) {
        let clamped = vol.clamp(0.0, 1.0);
        let cv = linear_to_channel_volumes(clamped, self.channels.max(1));
        let index = self.index;
        let is_output = self.is_output;
        let signal = make_signal();
        {
            let mut ml = self.mainloop.lock().unwrap();
            ml.lock();
            {
                let ctx = self.context.lock().unwrap();
                let mut introspect = ctx.introspect();
                if is_output {
                    introspect.set_sink_volume_by_index(index, &cv, Some(completion_cb(&signal)));
                } else {
                    introspect.set_source_volume_by_index(index, &cv, Some(completion_cb(&signal)));
                }
            }
            ml.unlock();
        }
        wait_for_signal(&signal);
        *self.volume.lock().unwrap() = clamped;
    }

    fn set_mute(&self, mute: bool) {
        let index = self.index;
        let is_output = self.is_output;
        let signal = make_signal();
        {
            let mut ml = self.mainloop.lock().unwrap();
            ml.lock();
            {
                let ctx = self.context.lock().unwrap();
                let mut introspect = ctx.introspect();
                if is_output {
                    introspect.set_sink_mute_by_index(index, mute, Some(completion_cb(&signal)));
                } else {
                    introspect.set_source_mute_by_index(index, mute, Some(completion_cb(&signal)));
                }
            }
            ml.unlock();
        }
        wait_for_signal(&signal);
        *self.muted.lock().unwrap() = mute;
    }

    fn get_mute(&self) -> bool {
        *self.muted.lock().unwrap()
    }
}

unsafe impl Send for LinuxDeviceSession {}
