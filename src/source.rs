use std::sync::mpsc::{
    self,
    RecvTimeoutError,
    SyncSender,
};
use std::sync::{
    Arc,
    Condvar,
    Mutex,
    PoisonError,
};
use std::thread::{
    self,
    JoinHandle,
};
use std::time::{
    Duration,
    Instant,
};

use crate::model::{
    Cause,
    Diagnostic,
    Operation,
    Problem,
    Snapshot,
    SourceId,
    SystemInfo,
    TelemetryState,
    deduplicate,
    limits,
};

pub const REDISCOVERY: Duration = Duration::from_secs(30);

pub trait Source {
    fn sample(
        &mut self,
        snapshot: &mut Snapshot,
    );
    fn describe(
        &self,
        _info: &mut SystemInfo,
    ) {
    }
    fn warm_up(
        &mut self,
        _deadline: Instant,
    ) {
    }
}

pub struct Collector {
    sources: Vec<Box<dyn Source>>,
    configuration: SystemInfo,
    described: Instant,
}
impl Collector {
    pub fn new(sources: Vec<Box<dyn Source>>) -> Self {
        let mut collector = Self {
            sources,
            configuration: SystemInfo::default(),
            described: Instant::now(),
        };
        collector.configuration = collector.describe();
        collector
    }
    fn describe(&self) -> SystemInfo {
        let mut info = SystemInfo::default();
        for source in &self.sources {
            source.describe(&mut info);
        }
        info.storage.truncate(limits::STORAGE);
        info
    }
    pub fn configuration(&self) -> &SystemInfo { &self.configuration }
    pub fn warm_up(
        &mut self,
        timeout: Duration,
    ) {
        let deadline = Instant::now() + timeout;
        for source in &mut self.sources {
            source.warm_up(deadline);
        }
    }
    pub fn sample(&mut self) -> Snapshot {
        if self.described.elapsed() >= REDISCOVERY {
            self.configuration = self.describe();
            self.described = Instant::now();
        }
        let mut snapshot = Snapshot::default();
        for source in &mut self.sources {
            source.sample(&mut snapshot);
        }
        snapshot.readings.truncate(limits::READINGS);
        snapshot.notices = deduplicate(std::mem::take(&mut snapshot.notices));
        snapshot.notices.truncate(limits::NOTICES);
        snapshot
    }
}

struct Stamped {
    part: Snapshot,
    sampled: Instant,
}
struct Slot {
    value: Mutex<Option<Stamped>>,
    filled: Condvar,
}
struct Worker {
    slot: Arc<Slot>,
    cancel: Option<SyncSender<()>>,
    handle: Option<JoinHandle<()>>,
    latest: Option<Stamped>,
}
impl Worker {
    fn poll(&mut self) -> Option<&Stamped> {
        if let Some(update) = self
            .slot
            .value
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            self.latest = Some(update);
        }
        self.latest.as_ref()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.try_send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub struct Background {
    source: SourceId,
    expiry: Duration,
    placeholder: Snapshot,
    worker: Result<Worker, Diagnostic>,
}
impl Background {
    pub fn spawn<S: Source>(
        source: SourceId,
        interval: Duration,
        expiry: Duration,
        placeholder: Snapshot,
        open: impl FnOnce() -> S + Send + 'static,
    ) -> Self {
        let slot = Arc::new(Slot {
            value: Mutex::new(None),
            filled: Condvar::new(),
        });
        let (cancel, stop) = mpsc::sync_channel(1);
        let shared = Arc::clone(&slot);
        let worker = thread::Builder::new()
            .name(source.to_string())
            .spawn(move || {
                let mut source = open();
                loop {
                    let mut part = Snapshot::default();
                    source.sample(&mut part);
                    *shared.value.lock().unwrap_or_else(PoisonError::into_inner) = Some(Stamped {
                        part,
                        sampled: Instant::now(),
                    });
                    shared.filled.notify_all();
                    match stop.recv_timeout(interval) {
                        Err(RecvTimeoutError::Timeout) => {}
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map(|handle| Worker {
                slot,
                cancel: Some(cancel),
                handle: Some(handle),
                latest: None,
            })
            .map_err(|error| {
                Diagnostic::unavailable(
                    source,
                    Operation::Initialize,
                    Cause::of(&error).context("worker thread"),
                )
            });
        Self {
            source,
            expiry,
            placeholder,
            worker,
        }
    }
}
impl Source for Background {
    fn sample(
        &mut self,
        snapshot: &mut Snapshot,
    ) {
        let state = |state| Diagnostic::new(self.source, Problem::State(state));
        let worker = match &mut self.worker {
            Ok(worker) => worker,
            Err(notice) => {
                snapshot.merge(self.placeholder.clone());
                return snapshot.notices.push(notice.clone());
            }
        };
        let stopped = worker.handle.as_ref().is_some_and(JoinHandle::is_finished);
        match worker.poll() {
            None => {
                snapshot.merge(self.placeholder.clone());
                snapshot.notices.push(state(TelemetryState::WarmingUp));
            }
            Some(update) if update.sampled.elapsed() > self.expiry => {
                snapshot.merge(update.part.clone().expire(TelemetryState::Stale));
                snapshot.notices.push(state(TelemetryState::Stale));
            }
            Some(update) => snapshot.merge(update.part.clone()),
        }
        if stopped {
            snapshot
                .notices
                .push(Diagnostic::new(self.source, Problem::Panicked));
        }
    }
    fn warm_up(
        &mut self,
        deadline: Instant,
    ) {
        let Ok(worker) = &mut self.worker else {
            return;
        };
        if worker.latest.is_some() {
            return;
        }
        let guard = worker
            .slot
            .value
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (mut guard, _) = worker
            .slot
            .filled
            .wait_timeout_while(
                guard,
                deadline.saturating_duration_since(Instant::now()),
                |value| value.is_none(),
            )
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(update) = guard.take() {
            worker.latest = Some(update);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{
        Duration,
        Instant,
    };

    use super::{
        Background,
        Collector,
        Source,
    };
    use crate::model::{
        GpuSample,
        History,
        Measurement,
        Problem,
        Snapshot,
        SourceId,
        TelemetryState,
    };

    struct Remote(mpsc::Receiver<f64>);
    impl Source for Remote {
        fn sample(
            &mut self,
            snapshot: &mut Snapshot,
        ) {
            snapshot.cpu_usage = Measurement::validated(self.0.recv().ok());
        }
    }
    fn sample(background: &mut Background) -> Snapshot {
        let mut snapshot = Snapshot::default();
        background.sample(&mut snapshot);
        snapshot
    }
    fn has_state(
        snapshot: &Snapshot,
        state: TelemetryState,
    ) -> bool {
        snapshot
            .notices
            .iter()
            .any(|notice| notice.problem == Problem::State(state))
    }

    #[test]
    fn background_warms_up_expires_and_recovers() {
        let (values, receiver) = mpsc::channel();
        let mut background = Background::spawn(
            SourceId::SENSOR,
            Duration::ZERO,
            Duration::from_millis(200),
            Snapshot::default(),
            move || Remote(receiver),
        );
        let warming = sample(&mut background);
        assert!(has_state(&warming, TelemetryState::WarmingUp));
        assert_eq!(values.send(10.0), Ok(()));
        background.warm_up(Instant::now() + Duration::from_secs(5));
        let fresh = sample(&mut background);
        assert_eq!(fresh.cpu_usage.numeric(), Some(10.0));
        assert!(fresh.notices.is_empty(), "A fresh result has no notice");
        std::thread::sleep(Duration::from_millis(300));
        let stale = sample(&mut background);
        assert_eq!(stale.cpu_usage, Measurement::Stale);
        assert!(has_state(&stale, TelemetryState::Stale));
        assert_eq!(values.send(20.0), Ok(()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while sample(&mut background).cpu_usage.numeric() != Some(20.0) {
            assert!(
                Instant::now() < deadline,
                "A new result must replace a stale one"
            );
            std::thread::yield_now();
        }
        drop(values);
    }

    struct ExtraGpu;
    impl Source for ExtraGpu {
        fn sample(
            &mut self,
            snapshot: &mut Snapshot,
        ) {
            snapshot.gpu.push(GpuSample {
                identity: "extra".into(),
                name: "Extra accelerator".into(),
                activity_percent: Measurement::validated(Some(42.0)),
                ..GpuSample::default()
            });
        }
    }
    #[test]
    fn a_new_source_needs_no_other_change() {
        let mut collector = Collector::new(vec![Box::new(ExtraGpu)]);
        let sample = collector.sample();
        let rendered = crate::ui::text(&crate::ui::rows(
            &crate::ui::View {
                configuration: collector.configuration(),
                sample: &sample,
                history: &History::new(),
                uptime: crate::model::Uptime::default(),
            },
            120,
            false,
            crate::ui::Output::Plain,
        ));
        assert!(rendered.contains("GPU 0 / Extra accelerator"));
        assert!(rendered.contains("42.0%"));
    }
}
