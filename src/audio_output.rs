use librespot_playback::audio_backend::{SinkError, SinkResult};
use rodio::{
    cpal::{
        self,
        traits::{DeviceTrait, HostTrait, StreamTrait},
    },
    mixer::{Mixer, MixerSource, mixer},
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_PROBE_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) enum OutputStream {
    Cpal(CpalStream),
    #[cfg(target_os = "macos")]
    Native(crate::macos_audio::OutputStream),
}

impl OutputStream {
    pub(crate) fn mixer(&self) -> &Mixer {
        match self {
            Self::Cpal(stream) => &stream.mixer,
            #[cfg(target_os = "macos")]
            Self::Native(stream) => stream.mixer(),
        }
    }
    /// Stops the device pulling samples. Audio it already holds stays queued
    /// and sounds as soon as it is thawed, instead of the device filling with
    /// silence that a resume would have to play out first.
    pub(crate) fn freeze(&self) -> bool {
        match self {
            Self::Cpal(stream) => stream.stream.pause().is_ok(),
            #[cfg(target_os = "macos")]
            Self::Native(_) => false,
        }
    }
    pub(crate) fn thaw(&self) {
        match self {
            Self::Cpal(stream) => {
                let _ = stream.stream.play();
            }
            #[cfg(target_os = "macos")]
            Self::Native(_) => {}
        }
    }
}

/// A cpal output fed by a rodio mixer. rodio's own stream offers no pause.
pub(crate) struct CpalStream {
    mixer: Mixer,
    stream: cpal::Stream,
}

fn feed<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut source: MixerSource,
    on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _| {
            for sample in data.iter_mut() {
                *sample = T::from_sample(source.next().unwrap_or(0.0));
            }
        },
        on_error,
        None,
    )
}

fn build_stream(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Option<CpalStream> {
    let (mixer, source) = mixer(supported.channels(), supported.sample_rate().0);
    let config = supported.config();
    use cpal::SampleFormat as Format;
    let stream = match supported.sample_format() {
        Format::F32 => feed::<f32>(device, &config, source, on_error),
        Format::F64 => feed::<f64>(device, &config, source, on_error),
        Format::I8 => feed::<i8>(device, &config, source, on_error),
        Format::I16 => feed::<i16>(device, &config, source, on_error),
        Format::I32 => feed::<i32>(device, &config, source, on_error),
        Format::I64 => feed::<i64>(device, &config, source, on_error),
        Format::U8 => feed::<u8>(device, &config, source, on_error),
        Format::U16 => feed::<u16>(device, &config, source, on_error),
        Format::U32 => feed::<u32>(device, &config, source, on_error),
        Format::U64 => feed::<u64>(device, &config, source, on_error),
        _ => return None,
    }
    .ok()?;
    stream.play().ok()?;
    Some(CpalStream { mixer, stream })
}

/// The device's default format first, then each format it supports, as
/// rodio's open_stream_or_fallback.
fn open_cpal(
    device: &cpal::Device,
    on_error: impl FnMut(cpal::StreamError) + Send + Clone + 'static,
) -> SinkResult<CpalStream> {
    let mut configs: Vec<_> = device.default_output_config().into_iter().collect();
    if let Ok(supported) = device.supported_output_configs() {
        configs.extend(supported.map(|range| range.with_max_sample_rate()));
    }
    configs
        .iter()
        .find_map(|config| build_stream(device, config, on_error.clone()))
        .ok_or_else(|| SinkError::ConnectionRefused("Cannot open audio output".into()))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OutputIssue {
    UsingDefault,
    Unavailable,
}

#[derive(Default)]
struct Selection {
    name: Option<String>,
    revision: u64,
    issue: Option<OutputIssue>,
    reported: Option<u64>,
    default_endpoint: Option<Option<String>>,
    using_default: bool,
    next_probe: Option<Instant>,
}

#[derive(Clone, Default)]
pub(crate) struct OutputChoice(Arc<Mutex<Selection>>);

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 512 && !name.chars().any(char::is_control)
}

#[cfg(any(feature = "desktop", test))]
pub(crate) fn devices() -> anyhow::Result<Vec<String>> {
    let mut names: Vec<_> = cpal::default_host()
        .output_devices()
        .map_err(|_| anyhow::anyhow!("Cannot list audio outputs"))?
        .filter_map(|device| device.name().ok())
        .filter(|name| valid_name(name))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    Ok(names)
}

impl OutputChoice {
    #[cfg(test)]
    pub(crate) fn defer_default_probe(&self) {
        // Synthetic PCM sinks must never probe or open physical audio hardware.
        self.0.lock().unwrap().next_probe = Some(Instant::now() + Duration::from_secs(3600));
    }
    pub fn request(&self, name: Option<String>) {
        if let Ok(mut selection) = self.0.lock()
            && selection.name != name
        {
            selection.name = name;
            selection.revision = selection.revision.wrapping_add(1);
            selection.issue = None;
            selection.reported = None;
            selection.default_endpoint = None;
            selection.using_default = false;
            selection.next_probe = None;
        }
    }
    #[cfg(test)]
    pub fn revision(&self) -> u64 {
        self.0.lock().map_or(0, |selection| selection.revision)
    }
    pub fn take_issue(&self) -> Option<OutputIssue> {
        self.0.lock().ok()?.issue.take()
    }
    pub fn retire_stream(&self) {
        if let Ok(mut selection) = self.0.lock() {
            selection.revision = selection.revision.wrapping_add(1);
            selection.issue = None;
            selection.reported = None;
        }
    }
    fn report(&self, revision: u64, issue: OutputIssue) {
        if let Ok(mut selection) = self.0.lock()
            && selection.revision == revision
            && selection.reported != Some(revision)
        {
            selection.issue = Some(issue);
            selection.reported = Some(revision);
        }
    }
    pub(crate) fn invalidate(&self, revision: u64) {
        if let Ok(mut selection) = self.0.lock()
            && selection.revision == revision
        {
            selection.revision = selection.revision.wrapping_add(1);
            selection.issue = Some(OutputIssue::Unavailable);
            selection.reported = Some(selection.revision);
        }
    }
    fn retry_stream(&self, revision: u64) {
        if let Ok(mut selection) = self.0.lock()
            && selection.revision == revision
        {
            #[cfg(not(test))]
            crate::native::diagnostic("audio output reported an error; reopening");
            selection.revision = selection.revision.wrapping_add(1);
            selection.issue = None;
            selection.reported = None;
            selection.next_probe = None;
        }
    }
    fn stream(&self, name: Option<&str>, revision: u64) -> SinkResult<OutputStream> {
        #[cfg(target_os = "macos")]
        {
            let choice = self.clone();
            match crate::macos_audio::OutputStream::open(name, move || {
                choice.retry_stream(revision)
            }) {
                Ok(stream) => return Ok(OutputStream::Native(stream)),
                Err(error) => {
                    log::warn!("Native macOS audio unavailable: {error}; using CoreAudio output")
                }
            }
        }
        let host = cpal::default_host();
        let device = if let Some(name) = name {
            host.output_devices()
                .ok()
                .and_then(|mut devices| {
                    devices.find(|device| device.name().ok().as_deref() == Some(name))
                })
                .ok_or_else(|| {
                    SinkError::ConnectionRefused("Selected audio output unavailable".into())
                })?
        } else {
            host.default_output_device()
                .ok_or_else(|| SinkError::ConnectionRefused("No audio output".into()))?
        };
        let choice = self.clone();
        let stream = open_cpal(&device, move |_| choice.retry_stream(revision))?;
        Ok(OutputStream::Cpal(stream))
    }
    pub fn open(&self, applied: Option<u64>) -> SinkResult<Option<(OutputStream, u64)>> {
        self.open_with(
            applied,
            Instant::now(),
            default_output_id,
            |name, revision| self.stream(name, revision),
        )
    }
    fn observe_default(&self, now: Instant, endpoint: impl FnOnce() -> Option<String>) {
        let revision = {
            let Ok(selection) = self.0.lock() else {
                return;
            };
            if (selection.name.is_some() && !selection.using_default)
                || selection.next_probe.is_some_and(|next| now < next)
            {
                return;
            }
            selection.revision
        };
        let endpoint = endpoint();
        if let Ok(mut selection) = self.0.lock()
            && selection.revision == revision
        {
            selection.next_probe = Some(now + DEFAULT_PROBE_INTERVAL);
            if selection
                .default_endpoint
                .as_ref()
                .is_some_and(|previous| previous != &endpoint)
            {
                #[cfg(not(test))]
                crate::native::diagnostic("default audio output changed; reopening");
                selection.revision = selection.revision.wrapping_add(1);
                selection.issue = None;
                selection.reported = None;
            }
            selection.default_endpoint = Some(endpoint);
        }
    }
    fn open_with<T>(
        &self,
        applied: Option<u64>,
        now: Instant,
        mut endpoint: impl FnMut() -> Option<String>,
        mut open: impl FnMut(Option<&str>, u64) -> SinkResult<T>,
    ) -> SinkResult<Option<(T, u64)>> {
        let mut attempts = 0;
        loop {
            self.observe_default(now, &mut endpoint);
            let (name, revision) = {
                let selection = self
                    .0
                    .lock()
                    .map_err(|_| SinkError::ConnectionRefused("Audio output unavailable".into()))?;
                if applied == Some(selection.revision) {
                    return Ok(None);
                }
                (selection.name.clone(), selection.revision)
            };
            attempts += 1;
            if attempts > 8 {
                self.report(revision, OutputIssue::Unavailable);
                return Err(SinkError::ConnectionRefused(
                    "Audio output could not stabilize".into(),
                ));
            }
            let mut using_default = name.is_none();
            let stream = match open(name.as_deref(), revision) {
                Ok(stream) => stream,
                Err(_) if name.is_some() => match open(None, revision) {
                    Ok(stream) => {
                        using_default = true;
                        self.report(revision, OutputIssue::UsingDefault);
                        stream
                    }
                    Err(error) => {
                        self.report(revision, OutputIssue::Unavailable);
                        return Err(error);
                    }
                },
                Err(error) => {
                    self.report(revision, OutputIssue::Unavailable);
                    return Err(error);
                }
            };
            {
                let mut selection = self
                    .0
                    .lock()
                    .map_err(|_| SinkError::ConnectionRefused("Audio output unavailable".into()))?;
                if selection.revision != revision {
                    if selection.issue == Some(OutputIssue::Unavailable) {
                        return Err(SinkError::ConnectionRefused(
                            "Audio output disconnected".into(),
                        ));
                    }
                    continue;
                }
                selection.using_default = using_default;
            }
            // A missing named device may have opened the default as fallback.
            self.observe_default(now, &mut endpoint);
            return Ok(Some((stream, revision)));
        }
    }
}

#[cfg(windows)]
fn default_output_id() -> Option<String> {
    use windows::Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender},
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            CoUninitialize,
        },
    };
    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized != RPC_E_CHANGED_MODE {
        initialized.ok().ok()?;
    }
    let _apartment = Apartment(initialized.is_ok());
    unsafe {
        let devices: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let id = device.GetId().ok()?;
        let value = id.to_string().ok();
        CoTaskMemFree(Some(id.0.cast()));
        value
    }
}

#[cfg(not(windows))]
fn default_output_id() -> Option<String> {
    cpal::default_host().default_output_device()?.name().ok()
}

/// Quiet, bounded output check. This never changes the saved device choice.
#[cfg(any(feature = "desktop", test))]
pub(crate) fn test_sound(name: Option<String>) -> anyhow::Result<()> {
    use rodio::Source;
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, Instant},
    };
    static TESTING: AtomicBool = AtomicBool::new(false);
    anyhow::ensure!(
        TESTING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok(),
        "An audio output test is already playing"
    );
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            TESTING.store(false, Ordering::Release);
        }
    }
    let _reset = Reset;
    anyhow::ensure!(
        name.as_ref().is_none_or(|name| valid_name(name)),
        "Invalid audio output"
    );
    let choice = OutputChoice::default();
    let stream = choice.stream(name.as_deref(), 0).map_err(|_| {
        anyhow::anyhow!(
            "Cannot test the selected audio output. Refresh outputs and choose an available device."
        )
    })?;
    let sink = rodio::Sink::connect_new(stream.mixer());
    for frequency in [440.0, 660.0] {
        sink.append(
            rodio::source::SineWave::new(frequency)
                .take_duration(Duration::from_millis(350))
                .amplify(0.04)
                .delay(Duration::from_millis(400)),
        );
    }
    let started = Instant::now();
    while !sink.empty() && started.elapsed() < Duration::from_secs(3) {
        anyhow::ensure!(
            choice.take_issue() != Some(OutputIssue::Unavailable),
            "Audio output disconnected during the test"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    anyhow::ensure!(sink.empty(), "Audio output did not consume the test sound");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_endpoint_change_reopens_active_playback() {
        let choice = OutputChoice::default();
        let now = Instant::now();
        let (_, first) = choice
            .open_with(None, now, || Some("speakers-id".into()), |_, _| Ok(()))
            .unwrap()
            .unwrap();
        let unchanged = choice
            .open_with(
                Some(first),
                now + DEFAULT_PROBE_INTERVAL,
                || Some("speakers-id".into()),
                |_, _| Ok(()),
            )
            .unwrap();
        assert!(unchanged.is_none());
        let switched = choice
            .open_with(
                Some(first),
                now + DEFAULT_PROBE_INTERVAL * 2,
                || Some("headphones-id".into()),
                |_, _| Ok(()),
            )
            .unwrap();
        assert!(
            switched.is_some(),
            "Playback kept the cached stream after the Windows default endpoint changed"
        );
        assert_eq!(choice.take_issue(), None);
    }
    #[test]
    fn a_driver_reset_reopens_before_reporting_output_unavailable() {
        let choice = OutputChoice::default();
        let now = Instant::now();
        let (_, first) = choice
            .open_with(None, now, || Some("speakers-id".into()), |_, _| Ok(()))
            .unwrap()
            .unwrap();
        choice.retry_stream(first);
        assert_eq!(choice.take_issue(), None);
        let (_, replacement) = choice
            .open_with(
                Some(first),
                now,
                || Some("headphones-id".into()),
                |_, _| Ok(()),
            )
            .unwrap()
            .unwrap();
        assert_ne!(replacement, first);
        choice.retry_stream(first);
        assert_eq!(
            choice.revision(),
            replacement,
            "Retired callbacks must not invalidate the new device"
        );
        assert_eq!(choice.take_issue(), None);
    }
    #[test]
    fn named_outputs_stay_pinned_but_default_fallback_follows_windows() {
        let now = Instant::now();
        for missing in [false, true] {
            let choice = OutputChoice::default();
            choice.request(Some("Headphones".into()));
            let open = |name: Option<&str>, _| {
                if missing && name.is_some() {
                    Err(SinkError::ConnectionRefused(
                        "Test device disconnected".into(),
                    ))
                } else {
                    Ok(())
                }
            };
            let (_, first) = choice
                .open_with(None, now, || Some("first-id".into()), open)
                .unwrap()
                .unwrap();
            choice.take_issue();
            let next = choice
                .open_with(
                    Some(first),
                    now + DEFAULT_PROBE_INTERVAL,
                    || Some("second-id".into()),
                    open,
                )
                .unwrap();
            assert_eq!(next.is_some(), missing);
        }
    }
    #[test]
    fn repeatedly_failing_streams_stop_reopening_and_report_failure() {
        let choice = OutputChoice::default();
        let mut opens = 0;
        let result = choice.open_with(
            None,
            Instant::now(),
            || Some("speakers-id".into()),
            |_, revision| {
                opens += 1;
                choice.retry_stream(revision);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert_eq!(opens, 8);
        assert_eq!(choice.take_issue(), Some(OutputIssue::Unavailable));
    }
    #[test]
    #[ignore = "Plays two quiet tones on the saved output"]
    fn live_saved_output_test_sound() {
        let settings = crate::config::Settings::load().unwrap();
        test_sound(settings.audio_output).unwrap();
        println!("Two quiet tones consumed by the selected output");
    }
    #[test]
    fn late_output_failures_cannot_replace_a_newer_selection() {
        let choice = OutputChoice::default();
        let old = choice.revision();
        choice.request(Some("Headphones".into()));
        choice.report(old, OutputIssue::Unavailable);
        choice.invalidate(old);
        assert_eq!(choice.revision(), old + 1);
        assert_eq!(choice.take_issue(), None);
        choice.report(choice.revision(), OutputIssue::UsingDefault);
        assert_eq!(choice.take_issue(), Some(OutputIssue::UsingDefault));
        choice.report(choice.revision(), OutputIssue::UsingDefault);
        assert_eq!(choice.take_issue(), None);
    }
    #[test]
    fn a_disconnected_stream_can_be_reopened_without_changing_the_preference() {
        let choice = OutputChoice::default();
        let old = choice.revision();
        choice.invalidate(old);
        assert_ne!(old, choice.revision());
        assert_eq!(choice.take_issue(), Some(OutputIssue::Unavailable));
        assert!(choice.0.lock().unwrap().name.is_none());
    }
    #[test]
    fn callbacks_from_a_retired_connection_cannot_pause_its_replacement() {
        let choice = OutputChoice::default();
        choice.request(Some("Headphones".into()));
        let old = choice.revision();
        choice.retire_stream();
        choice.invalidate(old);
        assert_eq!(choice.take_issue(), None);
        assert_eq!(choice.0.lock().unwrap().name.as_deref(), Some("Headphones"));
    }
    #[test]
    #[ignore = "Opens native audio outputs without playing audio"]
    fn live_output_selection_and_missing_device_fallback() {
        let available = devices().unwrap();
        assert!(
            !available.is_empty(),
            "Connect an audio output before testing"
        );
        let choice = OutputChoice::default();
        let default = cpal::default_host()
            .default_output_device()
            .and_then(|device| device.name().ok())
            .expect("System default output required");
        assert!(available.contains(&default));
        choice.request(Some(default));
        let (stream, revision) = choice.open(None).unwrap().unwrap();
        assert_eq!(choice.take_issue(), None);
        assert!(choice.open(Some(revision)).unwrap().is_none());
        drop(stream);
        choice.request(Some("Lyssna unavailable test output".into()));
        let (stream, _) = choice.open(None).unwrap().unwrap();
        assert_eq!(choice.take_issue(), Some(OutputIssue::UsingDefault));
        drop(stream);
    }

    #[test]
    #[ignore = "loads every core and opens the default output"]
    fn output_keeps_time_while_the_computer_is_busy() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        let device = cpal::default_host().default_output_device().unwrap();
        let supported = device.default_output_config().unwrap();
        let (late, worst) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let (late_cb, worst_cb) = (late.clone(), worst.clone());
        let mut last: Option<Instant> = None;
        let stream = device
            .build_output_stream::<f32, _, _>(
                &supported.config(),
                move |data: &mut [f32], _| {
                    data.fill(0.0);
                    let now = Instant::now();
                    if let Some(previous) = last.replace(now) {
                        let gap = now.duration_since(previous).as_micros() as u64;
                        worst_cb.fetch_max(gap, Ordering::Relaxed);
                        if gap > 25_000 {
                            late_cb.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                },
                |_| {},
                None,
            )
            .unwrap();
        stream.play().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let cores = std::thread::available_parallelism().map_or(8, |n| n.get());
        let load: Vec<_> = (0..cores * 2)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut x = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        x = std::hint::black_box(
                            x.wrapping_mul(6364136223846793005).wrapping_add(1),
                        );
                    }
                })
            })
            .collect();
        std::thread::sleep(Duration::from_secs(5));
        stop.store(true, Ordering::Relaxed);
        load.into_iter().for_each(|thread| thread.join().unwrap());
        let (late, worst) = (late.load(Ordering::Relaxed), worst.load(Ordering::Relaxed));
        eprintln!(
            "late callbacks {late}, worst gap {} ms",
            worst as f64 / 1000.0
        );
        assert_eq!(late, 0);
    }
}
