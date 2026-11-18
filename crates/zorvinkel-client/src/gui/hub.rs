//! Routes requests to background workers. Requests of one worker run strictly one after another.
use super::worker::{Reply, Worker};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use zorvinkel_application::setup::*;

/// Who a request belongs to, and therefore where its reply is delivered.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Owner {
    Discovery,
    Device(String),
}

/// Creates the backend that serves one device's own connection.
pub type BackendFactory = Box<dyn Fn() -> Box<dyn SetupBackend>>;

struct Job {
    owner: Owner,
    request: SetupRequest,
    background: bool,
}
pub struct Completed {
    pub owner: Owner,
    pub background: bool,
    pub reply: Reply,
}
struct Lane {
    worker: Worker,
    running: Option<(Owner, bool)>,
    queue: VecDeque<Job>,
}
impl Lane {
    fn new(
        backend: impl SetupBackend,
        wake: &Arc<dyn Fn() + Send + Sync>,
        delay: Duration,
    ) -> Self {
        let wake = wake.clone();
        Self {
            worker: Worker::new(backend, move || wake(), delay),
            running: None,
            queue: VecDeque::new(),
        }
    }
    fn idle(&self) -> bool {
        self.running.is_none() && self.queue.is_empty()
    }
    /// Starts the next queued request unless one is running.
    fn pump(&mut self) -> Option<Completed> {
        if self.running.is_some() {
            return None;
        }
        let job = self.queue.pop_front()?;
        match self.worker.submit(job.request) {
            Ok(()) => {
                self.running = Some((job.owner, job.background));
                None
            }
            Err(error) => Some(Completed {
                owner: job.owner,
                background: job.background,
                reply: Reply {
                    result: Err(error),
                    cancelled: false,
                },
            }),
        }
    }
}

/// Either one backend serves everything (a single connection at a time), or every device
/// gets its own backend and connections run side by side.
pub struct Hub {
    lanes: Vec<Lane>,
    device_lanes: HashMap<String, usize>,
    factory: Option<BackendFactory>,
    wake: Arc<dyn Fn() + Send + Sync>,
    delay: Duration,
}
impl Hub {
    #[cfg(any(
        all(any(unix, windows), feature = "security", feature = "ble-client"),
        test
    ))]
    pub fn single_connection(
        backend: impl SetupBackend,
        wake: impl Fn() + Send + Sync + 'static,
        delay: Duration,
    ) -> Self {
        Self::new(backend, None, Arc::new(wake), delay)
    }
    pub fn parallel_connections(
        discovery: impl SetupBackend,
        factory: BackendFactory,
        wake: impl Fn() + Send + Sync + 'static,
        delay: Duration,
    ) -> Self {
        Self::new(discovery, Some(factory), Arc::new(wake), delay)
    }
    fn new(
        backend: impl SetupBackend,
        factory: Option<BackendFactory>,
        wake: Arc<dyn Fn() + Send + Sync>,
        delay: Duration,
    ) -> Self {
        Self {
            lanes: vec![Lane::new(backend, &wake, delay)],
            device_lanes: HashMap::new(),
            factory,
            wake,
            delay,
        }
    }
    /// Connecting a device ends the connection of any other device.
    pub fn is_single_connection(&self) -> bool {
        self.factory.is_none()
    }
    fn lane_index(&mut self, owner: &Owner) -> usize {
        let (Owner::Device(handle), Some(factory)) = (owner, &self.factory) else {
            return 0;
        };
        if let Some(index) = self.device_lanes.get(handle) {
            return *index;
        }
        self.lanes
            .push(Lane::new(factory(), &self.wake, self.delay));
        self.device_lanes
            .insert(handle.clone(), self.lanes.len() - 1);
        self.lanes.len() - 1
    }
    /// True when nothing of this owner's worker is running or waiting.
    pub fn is_idle(&self, owner: &Owner) -> bool {
        let index = match (owner, &self.factory) {
            (Owner::Device(handle), Some(_)) => self.device_lanes.get(handle).copied(),
            _ => Some(0),
        };
        index.is_none_or(|index| self.lanes[index].idle())
    }
    pub fn submit(&mut self, owner: Owner, request: SetupRequest, background: bool) {
        let index = self.lane_index(&owner);
        self.lanes[index].queue.push_back(Job {
            owner,
            request,
            background,
        });
    }
    /// Collects finished requests and starts the queued ones.
    pub fn poll(&mut self) -> Vec<Completed> {
        let mut completed = Vec::new();
        for lane in &mut self.lanes {
            if let Some(reply) = lane.worker.poll()
                && let Some((owner, background)) = lane.running.take()
            {
                completed.push(Completed {
                    owner,
                    background,
                    reply,
                });
            }
            completed.extend(lane.pump());
        }
        completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zorvinkel_adapters::simulation::setup::SimulatedSetup;

    fn wait_for(hub: &mut Hub, count: usize) -> Vec<Completed> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut completed = Vec::new();
        while completed.len() < count {
            completed.extend(hub.poll());
            assert!(
                std::time::Instant::now() < deadline,
                "workers did not finish"
            );
            std::thread::yield_now();
        }
        completed
    }
    fn device(number: usize) -> Owner {
        Owner::Device(format!("simulation:device-{number}"))
    }
    fn connect(number: usize) -> SetupRequest {
        SetupRequest::Connect(format!("simulation:device-{number}"))
    }
    fn parallel_hub(count: usize, delay: Duration) -> Hub {
        Hub::parallel_connections(
            SimulatedSetup::with_device_count(count),
            Box::new(move || Box::new(SimulatedSetup::with_device_count(count).discovered())),
            || {},
            delay,
        )
    }

    #[test]
    fn requests_of_one_worker_run_in_order_and_reach_their_owner() {
        let mut hub =
            Hub::single_connection(SimulatedSetup::with_device_count(2), || {}, Duration::ZERO);
        hub.submit(Owner::Discovery, SetupRequest::Discover, false);
        hub.submit(device(1), connect(1), false);
        hub.submit(device(2), connect(2), true);
        let completed = wait_for(&mut hub, 3);
        let owners: Vec<_> = completed.iter().map(|c| c.owner.clone()).collect();
        assert_eq!(owners, [Owner::Discovery, device(1), device(2)]);
        assert!(completed[2].background && !completed[1].background);
        assert!(completed.iter().all(|c| c.reply.result.is_ok()));
    }
    #[test]
    fn single_connection_mode_serves_all_devices_from_one_backend() {
        let mut hub =
            Hub::single_connection(SimulatedSetup::with_device_count(2), || {}, Duration::ZERO);
        hub.submit(Owner::Discovery, SetupRequest::Discover, false);
        hub.submit(device(1), connect(1), false);
        hub.submit(device(2), connect(2), false);
        let completed = wait_for(&mut hub, 3);
        assert!(hub.is_single_connection());
        let last = completed.last().unwrap().reply.result.as_ref().unwrap();
        assert!(last.snapshot.status.is_some());
    }
    #[test]
    fn parallel_mode_serves_every_device_on_its_own_worker() {
        let mut hub = parallel_hub(30, Duration::from_millis(50));
        let started = std::time::Instant::now();
        for number in 1..=30 {
            hub.submit(device(number), connect(number), false);
        }
        let completed = wait_for(&mut hub, 30);
        // Serial execution would need 30 × 50 ms.
        assert!(started.elapsed() < Duration::from_millis(1000));
        assert!(!hub.is_single_connection());
        assert!(completed.iter().all(|c| {
            c.reply
                .result
                .as_ref()
                .is_ok_and(|update| update.error.is_none() && update.snapshot.status.is_some())
        }));
        assert!(hub.is_idle(&device(1)));
    }
    #[test]
    fn idle_reflects_running_and_queued_work() {
        let mut hub = parallel_hub(1, Duration::from_millis(20));
        assert!(hub.is_idle(&device(1)));
        hub.submit(device(1), connect(1), false);
        assert!(!hub.is_idle(&device(1)));
        wait_for(&mut hub, 1);
        assert!(hub.is_idle(&device(1)));
    }
}
