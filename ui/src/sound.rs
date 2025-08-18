
use std::ffi::CString;

use num_traits::Num;

trait SoundOut {
    fn buffer<Buffer, Sample>(&self, buf: Buffer, channels: u64)
    where
        Buffer: AsRef<[Sample]>,
        Sample: Num;
}

struct PulseAudioSoundOut {
    client: pulseaudio::Client,
}

impl PulseAudioSoundOut {
    fn new() -> Self {
        Self {
            client: pulseaudio::Client::from_env(CString::new("ReamioUI Pulse Backend").unwrap()).unwrap()
        }
    }
}

impl SoundOut for PulseAudioSoundOut {
    fn buffer<Buffer, Sample>(&self, buf: Buffer, channels: u64)
    where
        Buffer: AsRef<[Sample]>,
        Sample: Num {
        todo!()
    }
} 
