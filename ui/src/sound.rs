use std::{ffi::CString, io::Write};

use num_traits::Num;
use pa::AsPlaybackSource;
use pulseaudio as pa;
use pulseaudio::protocol as paproto;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

trait SoundOut {
    fn new_soundout() -> Self;

    fn buffer<Buffer, Sample>(&self, buf: Buffer, channels: u64)
    where
        Buffer: AsRef<[Sample]>,
        Sample: Num;
}

struct PulseAudioSoundOut {
    stream: pa::PlaybackStream,
    client: pa::Client,
    channel: SyncSender<f32>,
}

fn pa_pull_recv(buf: &mut [u8], recv: &mut Receiver<f32>) -> usize {
    let len = buf.len() as u64;
    let mut cursor = std::io::Cursor::new(buf);
    while cursor.position() < len
        && let Ok(buf) = recv.try_recv()
    {
        let Ok(_) = cursor.write(&buf.to_le_bytes()) else {
            break;
        };
    }
    cursor.position() as usize
}

impl SoundOut for PulseAudioSoundOut {
    fn new_soundout() -> Self {
        let (tx, mut rx) = sync_channel(2usize.pow(20));
        let client = pa::Client::from_env(CString::new("ReamioUI Pulse Backend").unwrap()).unwrap();
        let stream = smol::block_on(client.create_playback_stream(
            paproto::PlaybackStreamParams {
                channel_map: paproto::ChannelMap::stereo(),
                sample_spec: paproto::SampleSpec {
                    format: paproto::SampleFormat::Float32Le,
                    channels: 2,
                    sample_rate: 44100,
                },
                ..Default::default()
            },
            (move |buf: &mut [u8]| pa_pull_recv(buf, &mut rx)).as_playback_source(),
        ))
        .unwrap();
        Self {
            stream,
            client,
            channel: tx,
        }
    }

    fn buffer<Buffer, Sample>(&self, buf: Buffer, channels: u64)
    where
        Buffer: AsRef<[Sample]>,
        Sample: Num,
    {
        todo!()
    }
}
