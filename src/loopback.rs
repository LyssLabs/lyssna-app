use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{BLOB, COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Variant::VT_BLOB;
use windows::core::{HRESULT, Interface, Ref, implement};

const CHANNELS: u16 = 2;
const SAMPLE_RATE: u32 = 44_100;
const FLOAT: u16 = 3;
const CHUNK: usize = 1024 * CHANNELS as usize;

/// Feeds the playing bars from another program's audio, here Spotify
/// Soloist, which plays outside Lyssna's own output.
pub struct Levels {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Levels {
    pub fn follow(pid: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::Builder::new()
            .name("soloist-levels".into())
            .spawn(move || {
                if let Err(error) = capture(pid, &flag) {
                    log::info!("Soloist bars follow a pattern, not its audio: {error:#}");
                }
            })
            .ok();
        Self { stop, thread }
    }
}

impl Drop for Levels {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Activated(mpsc::Sender<()>);

impl IActivateAudioInterfaceCompletionHandler_Impl for Activated_Impl {
    fn ActivateCompleted(
        &self,
        _: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let _ = self.0.send(());
        Ok(())
    }
}

fn capture(pid: u32, stop: &AtomicBool) -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    let result = run(pid, stop);
    unsafe { CoUninitialize() };
    result
}

fn activate(pid: u32) -> Result<IAudioClient> {
    let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    // Borrows `params`; PropVariantClear on drop would free memory it
    // doesn't own.
    let variant = ManuallyDrop::new(PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_BLOB,
                wReserved1: 0,
                wReserved2: 0,
                wReserved3: 0,
                Anonymous: PROPVARIANT_0_0_0 {
                    blob: BLOB {
                        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                        pBlobData: (&raw mut params).cast(),
                    },
                },
            }),
        },
    });
    let (done, finished) = mpsc::channel();
    let handler: IActivateAudioInterfaceCompletionHandler = Activated(done).into();
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*variant),
            &handler,
        )
    }
    .context("start process capture")?;
    finished
        .recv_timeout(Duration::from_secs(5))
        .context("process capture did not start")?;
    let mut status = HRESULT(0);
    let mut client = None;
    unsafe { operation.GetActivateResult(&mut status, &mut client) }?;
    status.ok().context("process capture refused")?;
    let Some(client) = client else {
        bail!("process capture gave no client");
    };
    Ok(client.cast()?)
}

fn run(pid: u32, stop: &AtomicBool) -> Result<()> {
    let client = activate(pid)?;
    let format = WAVEFORMATEX {
        wFormatTag: FLOAT,
        nChannels: CHANNELS,
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    };
    unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM,
            2_000_000,
            0,
            &format,
            None,
        )
    }?;
    let reader: IAudioCaptureClient = unsafe { client.GetService() }?;
    unsafe { client.Start() }?;
    let mut samples = Vec::with_capacity(CHUNK * 2);
    let mut heard = None::<Instant>;
    let mut followed = false;
    while !stop.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(15));
        while unsafe { reader.GetNextPacketSize() }? > 0 {
            let mut data = std::ptr::null_mut();
            let mut frames = 0;
            let mut flags = 0;
            unsafe { reader.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }?;
            let count = frames as usize * usize::from(CHANNELS);
            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null() {
                samples.resize(samples.len() + count, 0.0);
            } else {
                samples.extend_from_slice(unsafe {
                    std::slice::from_raw_parts(data.cast::<f32>(), count)
                });
            }
            unsafe { reader.ReleaseBuffer(frames) }?;
        }
        if samples.len() < CHUNK {
            continue;
        }
        // Silence leaves the bars alone, so a song in Lyssna's own player
        // still drives them while Soloist is idle; they drop once when
        // Soloist goes quiet.
        if samples.iter().any(|sample| sample.abs() > 1e-5) {
            if !followed {
                followed = true;
                log::info!("Soloist bars follow its audio");
            }
            crate::audio::show_levels(&samples);
            heard = Some(Instant::now());
        } else if heard.is_some_and(|at| at.elapsed() > Duration::from_millis(150)) {
            crate::audio::show_levels(&[]);
            heard = None;
        }
        samples.clear();
    }
    let _ = unsafe { client.Stop() };
    Ok(())
}
