//! The default Windows output in low-latency shared mode (IAudioClient3).
//! The standard shared mode pulls audio every 10 ms, so a resume waits up to
//! 10 ms for the engine; drivers that allow a shorter period cut that wait.

use rodio::mixer::{Mixer, MixerSource, mixer};
use std::sync::mpsc;
use std::thread::JoinHandle;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        Media::Audio::{
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient3, IAudioRenderClient,
            IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, eConsole,
            eRender,
        },
        System::{
            Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
                CoUninitialize,
            },
            Threading::{
                AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
                INFINITE, SetEvent, WaitForMultipleObjects,
            },
        },
    },
    core::{GUID, w},
};

const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
const SUBTYPE_IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
/// Shortest engine period used, so a busy computer keeps some headroom.
const MIN_PERIOD_MS: u32 = 3;

enum Command {
    Play,
    Pause,
    Quit,
}

struct Event(HANDLE);
// SAFETY: an event handle may be signalled and waited on from any thread.
unsafe impl Send for Event {}
unsafe impl Sync for Event {}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub(crate) struct OutputStream {
    mixer: Mixer,
    commands: mpsc::Sender<Command>,
    wake: std::sync::Arc<Event>,
    thread: Option<JoinHandle<()>>,
}

impl OutputStream {
    pub(crate) fn mixer(&self) -> &Mixer {
        &self.mixer
    }
    fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            unsafe {
                let _ = SetEvent(self.wake.0);
            }
        }
    }
    pub(crate) fn freeze(&self) {
        self.send(Command::Pause);
    }
    pub(crate) fn thaw(&self) {
        self.send(Command::Play);
    }
    pub(crate) fn open(on_error: impl Fn() + Send + 'static) -> windows::core::Result<Self> {
        let wake = std::sync::Arc::new(Event(unsafe { CreateEventW(None, false, false, None)? }));
        let (commands, received) = mpsc::channel();
        let (ready, opened) = mpsc::channel();
        let thread_wake = wake.clone();
        let thread = std::thread::Builder::new()
            .name("lyssna-audio".into())
            .spawn(move || {
                let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
                match Render::open() {
                    Ok((render, mixer, source)) => {
                        let _ = ready.send(Ok(mixer));
                        if render.run(source, &received, &thread_wake).is_err() {
                            on_error();
                        }
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                    }
                }
                if initialized {
                    unsafe { CoUninitialize() };
                }
            })
            .map_err(|_| windows::core::Error::from_win32())?;
        match opened.recv() {
            Ok(Ok(mixer)) => Ok(Self {
                mixer,
                commands,
                wake,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => Err(windows::core::Error::from_win32()),
        }
    }
}

impl Drop for OutputStream {
    fn drop(&mut self) {
        self.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Render {
    client: IAudioClient3,
    render: IAudioRenderClient,
    event: Event,
    buffer_frames: u32,
    channels: usize,
}

fn is_float32(format: &WAVEFORMATEX) -> bool {
    let tag = format.wFormatTag;
    let bits = format.wBitsPerSample;
    if bits != 32 {
        return false;
    }
    match tag {
        WAVE_FORMAT_IEEE_FLOAT => true,
        WAVE_FORMAT_EXTENSIBLE => {
            // SAFETY: an extensible tag means the mix format is a
            // WAVEFORMATEXTENSIBLE; it is packed, so read it unaligned.
            let extensible = unsafe {
                std::ptr::read_unaligned(
                    (format as *const WAVEFORMATEX).cast::<WAVEFORMATEXTENSIBLE>(),
                )
            };
            let sub_format = extensible.SubFormat;
            sub_format == SUBTYPE_IEEE_FLOAT
        }
        _ => false,
    }
}

/// The smallest multiple of the engine's step that is at least
/// `MIN_PERIOD_MS`, within what the driver allows.
fn period_frames(rate: u32, fundamental: u32, min: u32, max: u32) -> u32 {
    let floor = (rate * MIN_PERIOD_MS).div_ceil(1000).max(min);
    let step = fundamental.max(1);
    (floor.div_ceil(step) * step).clamp(min, max.max(min))
}

impl Render {
    fn open() -> windows::core::Result<(Self, Mixer, MixerSource)> {
        unsafe {
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = devices.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let client: IAudioClient3 = device.Activate(CLSCTX_ALL, None)?;
            let format = client.GetMixFormat()?;
            let opened = (|| {
                let mix = &*format;
                if !is_float32(mix) {
                    return Err(windows::core::Error::from_hresult(
                        windows::Win32::Foundation::E_NOTIMPL,
                    ));
                }
                let (mut default, mut fundamental, mut min, mut max) = (0, 0, 0, 0);
                client.GetSharedModeEnginePeriod(
                    format,
                    &mut default,
                    &mut fundamental,
                    &mut min,
                    &mut max,
                )?;
                let (channels, rate) = (mix.nChannels, mix.nSamplesPerSec);
                let period = period_frames(rate, fundamental, min, max);
                client.InitializeSharedAudioStream(
                    AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
                    period,
                    format,
                    None,
                )?;
                #[cfg(not(test))]
                crate::native::diagnostic(&format!(
                    "audio output low latency: {period} frame period (driver {min}-{max}, standard {default}) at {rate} Hz"
                ));
                #[cfg(test)]
                eprintln!("period {period} (driver {min}-{max}, standard {default}) at {rate} Hz");
                Ok((channels, rate))
            })();
            CoTaskMemFree(Some(format.cast()));
            let (channels, rate) = opened?;
            let event = Event(CreateEventW(None, false, false, None)?);
            client.SetEventHandle(event.0)?;
            let buffer_frames = client.GetBufferSize()?;
            let render: IAudioRenderClient = client.GetService()?;
            let (mixer, source) = mixer(channels, rate);
            Ok((
                Self {
                    client,
                    render,
                    event,
                    buffer_frames,
                    channels: channels as usize,
                },
                mixer,
                source,
            ))
        }
    }

    fn fill(&self, source: &mut MixerSource) -> windows::core::Result<()> {
        unsafe {
            let free = self.buffer_frames - self.client.GetCurrentPadding()?;
            if free == 0 {
                return Ok(());
            }
            let data = self.render.GetBuffer(free)?.cast::<f32>();
            let samples = std::slice::from_raw_parts_mut(data, free as usize * self.channels);
            for sample in samples.iter_mut() {
                *sample = source.next().unwrap_or(0.0);
            }
            self.render.ReleaseBuffer(free, 0)
        }
    }

    fn run(
        &self,
        mut source: MixerSource,
        commands: &mpsc::Receiver<Command>,
        wake: &Event,
    ) -> windows::core::Result<()> {
        let mut task_index = 0;
        let task = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) };
        let result = (|| {
            self.fill(&mut source)?;
            unsafe { self.client.Start()? };
            let mut running = true;
            loop {
                let signalled =
                    unsafe { WaitForMultipleObjects(&[self.event.0, wake.0], false, INFINITE) };
                while let Ok(command) = commands.try_recv() {
                    match command {
                        Command::Play if !running => {
                            unsafe { self.client.Start()? };
                            running = true;
                        }
                        Command::Pause if running => {
                            unsafe { self.client.Stop()? };
                            running = false;
                        }
                        Command::Quit => return Ok(()),
                        _ => {}
                    }
                }
                if running && signalled == WAIT_OBJECT_0 {
                    self.fill(&mut source)?;
                }
            }
        })();
        if let Ok(task) = task {
            unsafe {
                let _ = AvRevertMmThreadCharacteristics(task);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "plays a quiet tone on the default output"]
    fn opens_and_plays_on_the_default_output() {
        use rodio::Source;
        let stream = OutputStream::open(|| panic!("output failed")).unwrap();
        let tone = rodio::source::SineWave::new(440.0)
            .amplify(0.02)
            .take_duration(std::time::Duration::from_millis(400));
        stream.mixer().add(tone);
        std::thread::sleep(std::time::Duration::from_millis(150));
        stream.freeze();
        std::thread::sleep(std::time::Duration::from_millis(100));
        stream.thaw();
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    #[test]
    fn period_is_the_shortest_step_multiple_past_the_floor() {
        // 48 kHz, 128-frame steps: 3 ms is 144 frames, so 256.
        assert_eq!(period_frames(48_000, 128, 128, 480), 256);
        // 32-frame steps reach 160 (3.3 ms).
        assert_eq!(period_frames(48_000, 32, 32, 480), 160);
        // Drivers without low latency offer only the standard 10 ms.
        assert_eq!(period_frames(48_000, 480, 480, 480), 480);
        assert_eq!(period_frames(44_100, 441, 441, 441), 441);
    }
}
