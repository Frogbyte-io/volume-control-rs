pub trait Session: Send {
    fn get_name(&self) -> String;
    fn get_volume(&self) -> f32;
    fn set_volume(&self, vol: f32);
    fn get_mute(&self) -> bool;
    fn set_mute(&self, mute: bool);
}
