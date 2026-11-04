pub use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

pub use vela_server::{
    audio::PcmF32Mono,
    providers::{TtsBinding, TtsError, TtsProvider, TtsStream, TtsSynthesisRequest, TtsWorker},
    workers::{TtsLease, TtsWorkerEvent, TtsWorkerRuntime, WorkerRuntimeConfig},
};

pub fn binding(voice: &str) -> TtsBinding {
    TtsBinding {
        voice: voice.into(),
        language: "vi-VN".into(),
    }
}

pub struct BlockingTts {
    pub entered: mpsc::Sender<()>,
    pub release: std::sync::Mutex<mpsc::Receiver<()>>,
}

impl TtsProvider for BlockingTts {
    fn adapter(&self) -> &'static str {
        "blocking"
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(PcmF32Mono::new(vec![0.25; 96], 48_000))
    }
}

pub struct StreamingTts;

impl TtsProvider for StreamingTts {
    fn adapter(&self) -> &'static str {
        "streaming"
    }

    fn synthesize_stream(
        &self,
        _: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        on_pcm(PcmF32Mono::new(vec![0.1; 96], 48_000))?;
        on_pcm(PcmF32Mono::new(vec![0.2; 96], 48_000))
    }
}

pub struct StatefulTts;

impl TtsProvider for StatefulTts {
    fn adapter(&self) -> &'static str {
        "stateful"
    }

    fn open_stream(&self) -> Option<Box<dyn TtsStream>> {
        Some(Box::new(StatefulStream { segments: 0 }))
    }
}

pub struct StatefulStream {
    pub segments: usize,
}

impl TtsStream for StatefulStream {
    fn synthesize(
        &mut self,
        _: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.segments += 1;
        on_pcm(PcmF32Mono::new(vec![self.segments as f32; 96], 48_000))
    }
}

pub struct CountedWorkerProvider(pub Arc<AtomicUsize>);

impl TtsProvider for CountedWorkerProvider {
    fn adapter(&self) -> &'static str {
        "counted-worker"
    }
    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(CountedWorker))
    }
}

pub struct CountedWorker;

impl TtsWorker for CountedWorker {
    fn synthesize(
        &mut self,
        _: &TtsSynthesisRequest,
        _: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        on_pcm(PcmF32Mono::new(vec![0.1; 96], 48_000))
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

pub struct BindingRecordingProvider(pub Arc<std::sync::Mutex<Vec<String>>>);

impl TtsProvider for BindingRecordingProvider {
    fn adapter(&self) -> &'static str {
        "binding-recording"
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(BindingRecordingWorker(Arc::clone(&self.0))))
    }
}

pub struct BindingRecordingWorker(pub Arc<std::sync::Mutex<Vec<String>>>);

impl TtsWorker for BindingRecordingWorker {
    fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        _: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.0.lock().unwrap().push(request.selection.voice.clone());
        on_pcm(PcmF32Mono::new(vec![0.1; 96], 48_000))
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

pub struct BurstingWorkerProvider(pub mpsc::Sender<()>);

impl TtsProvider for BurstingWorkerProvider {
    fn adapter(&self) -> &'static str {
        "bursting-worker"
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(BurstingWorker(self.0.clone())))
    }
}

pub struct BurstingWorker(pub mpsc::Sender<()>);

impl TtsWorker for BurstingWorker {
    fn synthesize(
        &mut self,
        _: &TtsSynthesisRequest,
        _: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        on_pcm(PcmF32Mono::new(vec![0.1; 2_880], 48_000))?;
        self.0.send(()).unwrap();
        on_pcm(PcmF32Mono::new(vec![0.2; 2_880], 48_000))
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

pub fn wait_for_finish(runtime: &TtsWorkerRuntime, lease: TtsLease) {
    loop {
        if matches!(runtime.poll(lease).unwrap(), Some(TtsWorkerEvent::Finished)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
