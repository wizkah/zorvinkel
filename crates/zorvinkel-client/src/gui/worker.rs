use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use zorvinkel_application::setup::*;

struct Work {
    request: SetupRequest,
    cancel: Arc<AtomicBool>,
}
pub struct Reply {
    pub result: Result<SetupUpdate, SetupError>,
    pub cancelled: bool,
}
pub struct Worker {
    sender: Option<SyncSender<Work>>,
    receiver: Receiver<Reply>,
    thread: Option<JoinHandle<()>>,
    active: Option<Arc<AtomicBool>>,
}
impl Worker {
    pub fn new(
        mut backend: impl SetupBackend,
        wake: impl Fn() + Send + 'static,
        delay: Duration,
    ) -> Self {
        let (sender, requests) = mpsc::sync_channel::<Work>(1);
        let (replies, receiver) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            while let Ok(work) = requests.recv() {
                thread::sleep(delay);
                if replies.send(process(&mut backend, work)).is_err() {
                    break;
                }
                wake();
            }
        });
        Self {
            sender: Some(sender),
            receiver,
            thread: Some(thread),
            active: None,
        }
    }
    pub fn busy(&self) -> bool {
        self.active.is_some()
    }
    pub fn submit(&mut self, request: SetupRequest) -> Result<(), SetupError> {
        if self.busy() {
            return Err(SetupError::Unavailable);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.sender
            .as_ref()
            .ok_or(SetupError::Unavailable)?
            .try_send(Work {
                request,
                cancel: cancel.clone(),
            })
            .map_err(|_| SetupError::Unavailable)?;
        self.active = Some(cancel);
        Ok(())
    }
    pub fn cancel(&self) {
        if let Some(cancel) = &self.active {
            cancel.store(true, Ordering::Release);
        }
    }
    pub fn poll(&mut self) -> Option<Reply> {
        match self.receiver.try_recv() {
            Ok(reply) => {
                self.active = None;
                Some(reply)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) if self.active.take().is_some() => Some(Reply {
                result: Err(SetupError::Unavailable),
                cancelled: false,
            }),
            Err(_) => None,
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        self.sender.take();
        // Release the reply receiver before joining so shutdown cannot block on a full queue.
        let (_, empty) = mpsc::channel();
        let receiver = std::mem::replace(&mut self.receiver, empty);
        drop(receiver);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            eprintln!("Setup worker terminated unexpectedly");
        }
    }
}
fn process(backend: &mut impl SetupBackend, work: Work) -> Reply {
    let cancelled = work.cancel.load(Ordering::Acquire);
    let request = if cancelled {
        SetupRequest::Refresh
    } else {
        work.request
    };
    Reply {
        result: backend.handle(request),
        cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zorvinkel_adapters::simulation::setup::SimulatedSetup;
    #[test]
    fn cancellation_before_dispatch_preserves_actual_state() {
        let mut backend = SimulatedSetup::default();
        backend.handle(SetupRequest::Discover).unwrap();
        let reply = process(
            &mut backend,
            Work {
                request: SetupRequest::Connect("simulation:device-1".into()),
                cancel: Arc::new(AtomicBool::new(true)),
            },
        );
        assert!(reply.cancelled);
        assert!(reply.result.unwrap().snapshot.status.is_none());
    }
    #[test]
    fn cancellation_does_not_poison_later_connections() {
        let mut backend = SimulatedSetup::default();
        backend.handle(SetupRequest::Discover).unwrap();
        let work = |request, cancelled| Work {
            request,
            cancel: Arc::new(AtomicBool::new(cancelled)),
        };
        let cancelled = process(
            &mut backend,
            work(SetupRequest::Connect("simulation:device-1".into()), true),
        );
        assert!(cancelled.cancelled);
        for request in [
            SetupRequest::Connect("simulation:device-1".into()),
            SetupRequest::Disconnect,
            SetupRequest::Connect("simulation:device-1".into()),
        ] {
            let connected = matches!(&request, SetupRequest::Connect(_));
            let reply = process(&mut backend, work(request, false));
            assert!(!reply.cancelled);
            let update = reply.result.unwrap();
            assert!(update.error.is_none());
            assert_eq!(update.snapshot.status.is_some(), connected);
        }
    }
}
