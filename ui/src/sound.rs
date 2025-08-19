use std::{
    any::{Any, TypeId},
    ffi::CString,
    io::Write,
    ops::{Div, Sub},
};

use num_traits::{AsPrimitive, Num, ToPrimitive};
use pa::AsPlaybackSource;
use pulseaudio as pa;
use pulseaudio::protocol as paproto;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

// Macro for doing conversions from a number format to f32 for range 
// mapping, before going back to Self::ExpectedFormat
macro_rules! match_conv {
    ($buf:expr => $($ty:ty),+) => {
        if TypeId::of::<Sample>() == TypeId::of::<Self::ExpectedFormat>() {
            $buf.into_iter().map(|x| x.as_()).collect::<Vec<Self::ExpectedFormat>>()
        }
        $(
             else if TypeId::of::<Sample>() == TypeId::of::<$ty>() {
                 $buf
                     .into_iter()
                     .map(|x|
                         AsPrimitive::<Self::ExpectedFormat>::as_(((x - <$ty>::LOW_BOUND as f32) /
                             (<$ty>::HIGH_BOUND as f32 - <$ty>::LOW_BOUND as f32)).round()))
                     .collect::<Vec<Self::ExpectedFormat>>()
             }
        )*
        else {
            panic!("undefined auto convert impl for type")
        }
    }
}

// Trait for describing audio playback backends
trait SoundOut
where
    f32: AsPrimitive<<Self as SoundOut>::ExpectedFormat>,
{
    type ExpectedFormat: Copy + 'static;

    fn new_soundout() -> Self;

    fn buffer<Buffer>(&self, buf: Buffer, channels: u64) -> Result<usize, ()>
    where
        Buffer: AsRef<[Self::ExpectedFormat]>;

    fn conv_and_buffer<Buffer, Sample>(&self, buf: Buffer, channels: u64) -> Result<usize, ()>
    where
        Buffer: AsRef<[Sample]>,
        Sample: AsPrimitive<f32> + SampleType,
    {
        let buf: Vec<f32> = buf.as_ref().to_vec().into_iter().map(|x| x.as_()).collect();
        let buf = match_conv!(buf => u32, i32, u16, i16, u8, i8, f32, f64);

        self.buffer(buf, channels)
    }
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
    type ExpectedFormat = f32;

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

    fn buffer<Buffer>(&self, buf: Buffer, channels: u64) -> Result<usize, ()>
    where
        Buffer: AsRef<[Self::ExpectedFormat]>,
    {
        todo!()
    }
}

trait SampleType {
    const LOW_BOUND: Self;
    const HIGH_BOUND: Self;
}

macro_rules! gen_sampletype {
    ($($x:ty),*) => {
        $(
            impl SampleType for $x {
                const LOW_BOUND: Self = <$x>::MIN;
                const HIGH_BOUND: Self = <$x>::MAX;
            }
        )*
    };

    ($($x:ty | ($low:expr, $high:expr)),*) => {
        $(
            impl SampleType for $x {
                const LOW_BOUND: Self = $low;
                const HIGH_BOUND: Self = $high;
            }
        )*
    };
}

gen_sampletype!(i32, u32, i16, u16, i8, u8);
gen_sampletype!(f32 | (-1.0, 1.0), f64 | (-1.0, 1.0));
