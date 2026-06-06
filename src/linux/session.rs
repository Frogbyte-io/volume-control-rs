use std::sync::{Arc, Mutex};

use libpulse_binding as pulse;
use pulse::mainloop::threaded::Mainloop;
use pulse::context::Context;
use pulse::volume::{ChannelVolumes, Volume};

use crate::Session;

fn linear_to_channel_volumes(vol: f32, channels: u8) -> ChannelVolumes {
    let pa_vol = Volume::from_linear(vol as f64);
    let mut cv = ChannelVolumes::default();
    cv.set(channels as u32, pa_vol);
    cv
}

pub struct LinuxApplicationSession {
    pub index: u32,
    pub name: String,
    pub channels: u8,
    volume: Mutex<f32>,
    muted: Mutex<bool>,
    mainloop: Arc<Mainloop>,
    context: Arc<Mutex<Context>>,
}

impl LinuxApplicationSession {
    pub fn new(
        index: u32,
        name: String,
        channels: u8,
        volume: f32,
        muted: bool,
        mainloop: Arc<Mainloop>,
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
        {
            let ml = &self.mainloop;
            ml.lock();
            self.context.lock().unwrap().introspect().set_sink_input_volume(index, &cv, None);
            ml.unlock();
        }
        *self.volume.lock().unwrap() = clamped;
    }

    fn set_mute(&self, mute: bool) {
        let index = self.index;
        {
            let ml = &self.mainloop;
            ml.lock();
            self.context.lock().unwrap().introspect().set_sink_input_mute(index, mute, None);
            ml.unlock();
        }
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
    mainloop: Arc<Mainloop>,
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
        mainloop: Arc<Mainloop>,
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
        {
            let ml = &self.mainloop;
            ml.lock();
            let introspect = self.context.lock().unwrap().introspect();
            if is_output {
                introspect.set_sink_volume_by_index(index, &cv, None);
            } else {
                introspect.set_source_volume_by_index(index, &cv, None);
            }
            ml.unlock();
        }
        *self.volume.lock().unwrap() = clamped;
    }

    fn set_mute(&self, mute: bool) {
        let index = self.index;
        let is_output = self.is_output;
        {
            let ml = &self.mainloop;
            ml.lock();
            let introspect = self.context.lock().unwrap().introspect();
            if is_output {
                introspect.set_sink_mute_by_index(index, mute, None);
            } else {
                introspect.set_source_mute_by_index(index, mute, None);
            }
            ml.unlock();
        }
        *self.muted.lock().unwrap() = mute;
    }

    fn get_mute(&self) -> bool {
        *self.muted.lock().unwrap()
    }
}

unsafe impl Send for LinuxDeviceSession {}
