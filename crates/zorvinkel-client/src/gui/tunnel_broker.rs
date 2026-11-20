//! Brings streams of the Bluetooth tunnel (ADR-031) to the terminal's connector.
//!
//! The terminal connects and reconnects on its own runtime, but the Bluetooth connection belongs
//! to the device's worker. The connector therefore leaves a request here; the window loop turns it
//! into a worker job (queued behind whatever the worker does, such as a session renewal) and
//! hands the resulting stream back.
use super::terminal_page::Route;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::oneshot,
    time::{sleep, timeout},
};
use zorvinkel_terminal::{ByteStream, ConnectError, Dialer, Dialing};

/// Longest wait for a stream, including the time the worker needs for earlier jobs.
const DIAL_TIMEOUT: Duration = Duration::from_secs(45);
/// The device carries one tunnel at a time; the previous one may still be closing.
const BUSY_TRIES: usize = 6;
const BUSY_PAUSE: Duration = Duration::from_millis(700);

#[derive(Debug, PartialEq, Eq)]
pub enum TunnelFailure {
    /// The device carries another tunnel right now.
    Busy,
    /// The device does not offer the terminal over Bluetooth (not supported or switched off).
    NotOffered,
    /// The connection to the device did not deliver a tunnel; trying again may help.
    Unreachable,
}
type Answer = Result<Box<dyn ByteStream>, TunnelFailure>;

#[derive(Default)]
pub struct TunnelBroker {
    asks: Mutex<VecDeque<oneshot::Sender<Answer>>>,
}
impl TunnelBroker {
    fn queue(&self) -> std::sync::MutexGuard<'_, VecDeque<oneshot::Sender<Answer>>> {
        self.asks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    fn ask(&self) -> oneshot::Receiver<Answer> {
        let (sender, receiver) = oneshot::channel();
        self.queue().push_back(sender);
        receiver
    }
    #[cfg(test)]
    pub fn ask_for_test(&self) -> oneshot::Receiver<Answer> {
        self.ask()
    }
    /// Someone waits for a stream. Requests whose connector gave up are dropped first.
    pub fn has_asks(&self) -> bool {
        let mut queue = self.queue();
        queue.retain(|ask| !ask.is_closed());
        !queue.is_empty()
    }
    /// Answers the oldest request that still has a listener.
    pub fn answer(&self, mut answer: Answer) {
        let mut queue = self.queue();
        while let Some(ask) = queue.pop_front() {
            match ask.send(answer) {
                Ok(()) => return,
                Err(returned) => answer = returned,
            }
        }
    }
}

/// The connector's way to a tunnel stream.
pub struct BluetoothDialer {
    pub broker: Arc<TunnelBroker>,
    /// Wakes the window loop, which only runs when something asks it to.
    pub wake: Arc<dyn Fn() + Send + Sync>,
}
impl Dialer for BluetoothDialer {
    fn dial(&self) -> Dialing<'_> {
        Box::pin(async move {
            for attempt in 0..BUSY_TRIES {
                let answer = self.broker.ask();
                (self.wake)();
                match timeout(DIAL_TIMEOUT, answer).await {
                    Ok(Ok(Ok(stream))) => return Ok(stream),
                    Ok(Ok(Err(TunnelFailure::Busy))) if attempt + 1 < BUSY_TRIES => {
                        sleep(BUSY_PAUSE).await;
                    }
                    Ok(Ok(Err(TunnelFailure::NotOffered))) => {
                        return Err(ConnectError::Refused(
                            "the device does not offer the terminal over Bluetooth".into(),
                        ));
                    }
                    _ => {
                        return Err(ConnectError::Network("no tunnel over Bluetooth".into()));
                    }
                }
            }
            Err(ConnectError::Network("the device is busy".into()))
        })
    }
}

/// The way to the device as the window currently sees it; the terminal keeps it up to date.
pub type LiveRoute = Arc<Mutex<Option<Route>>>;
const ROUTE_WATCH: Duration = Duration::from_millis(500);

/// Picks the way to the device anew for every connection attempt: after the device changed
/// from Bluetooth to the network (or back), a reconnect has to follow it.
pub struct RoutedDialer {
    pub route: LiveRoute,
    pub wake: Arc<dyn Fn() + Send + Sync>,
}
impl RoutedDialer {
    fn current(&self) -> Option<Route> {
        self.route
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
    /// Returns once the route is no longer `route`.
    async fn changed_from(&self, route: &Route) {
        loop {
            sleep(ROUTE_WATCH).await;
            match self.current() {
                Some(now) if now.same_way(route) => {}
                _ => return,
            }
        }
    }
    async fn dial_route(&self, route: &Route) -> Result<Box<dyn ByteStream>, ConnectError> {
        match route {
            Route::Ip(address) => {
                let socket = std::net::SocketAddr::new(*address, SSH_PORT);
                let stream = tokio::net::TcpStream::connect(socket)
                    .await
                    .map_err(|error| ConnectError::Network(error.to_string()))?;
                let _ = stream.set_nodelay(true);
                Ok(Box::new(stream))
            }
            Route::Bluetooth(broker) => {
                BluetoothDialer {
                    broker: broker.clone(),
                    wake: self.wake.clone(),
                }
                .dial()
                .await
            }
        }
    }
}
impl Dialer for RoutedDialer {
    fn dial(&self) -> Dialing<'_> {
        Box::pin(async move {
            // The route may change while an attempt waits; then the attempt starts over.
            for _ in 0..ROUTE_SWITCHES {
                let Some(route) = self.current() else {
                    return Err(ConnectError::Network("no way to the device".into()));
                };
                tokio::select! {
                    result = self.dial_route(&route) => return result,
                    () = self.changed_from(&route) => {}
                }
            }
            Err(ConnectError::Network(
                "the way to the device keeps changing".into(),
            ))
        })
    }
}
const ROUTE_SWITCHES: usize = 5;
const SSH_PORT: u16 = 22;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    fn stream() -> Box<dyn ByteStream> {
        Box::new(duplex(64).0)
    }
    fn new_dialer() -> (Arc<TunnelBroker>, BluetoothDialer) {
        let broker = Arc::new(TunnelBroker::default());
        let dialer = BluetoothDialer {
            broker: broker.clone(),
            wake: Arc::new(|| {}),
        };
        (broker, dialer)
    }
    async fn wait_for_ask(broker: &TunnelBroker) {
        while !broker.has_asks() {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn a_dial_gets_the_stream_the_window_loop_answers_with() {
        let (broker, dialer) = new_dialer();
        let dialing = tokio::spawn(async move { dialer.dial().await.is_ok() });
        wait_for_ask(&broker).await;
        broker.answer(Ok(stream()));
        assert!(dialing.await.unwrap());
        assert!(!broker.has_asks());
    }
    #[tokio::test]
    async fn a_busy_device_is_tried_again_and_a_missing_offer_is_final() {
        let (broker, dialer) = new_dialer();
        let dialing = tokio::spawn(async move { dialer.dial().await });
        wait_for_ask(&broker).await;
        broker.answer(Err(TunnelFailure::Busy));
        wait_for_ask(&broker).await;
        broker.answer(Ok(stream()));
        assert!(dialing.await.unwrap().is_ok());

        let (broker, dialer) = new_dialer();
        let dialing = tokio::spawn(async move { dialer.dial().await });
        wait_for_ask(&broker).await;
        broker.answer(Err(TunnelFailure::NotOffered));
        let error = dialing.await.unwrap().err().unwrap();
        assert!(!error.is_retryable());
    }
    #[tokio::test]
    async fn an_unreachable_device_is_a_retryable_failure() {
        let (broker, dialer) = new_dialer();
        let dialing = tokio::spawn(async move { dialer.dial().await });
        wait_for_ask(&broker).await;
        broker.answer(Err(TunnelFailure::Unreachable));
        assert!(dialing.await.unwrap().err().unwrap().is_retryable());
    }
    #[tokio::test]
    async fn a_dial_waiting_for_bluetooth_follows_the_device_to_the_network() {
        let broker = Arc::new(TunnelBroker::default());
        let route: LiveRoute = Arc::new(Mutex::new(Some(Route::Bluetooth(broker.clone()))));
        let dialer = Arc::new(RoutedDialer {
            route: route.clone(),
            wake: Arc::new(|| {}),
        });
        let waiting = dialer.clone();
        let dialing = tokio::spawn(async move { waiting.dial().await.err() });
        wait_for_ask(&broker).await;
        // Nothing listens on this address, so the attempt over the network fails quickly.
        *route.lock().unwrap() = Some(Route::Ip("127.0.0.1".parse().unwrap()));
        let error = tokio::time::timeout(Duration::from_secs(10), dialing)
            .await
            .expect("the dial left the Bluetooth wait")
            .unwrap();
        assert!(error.is_none() || error.unwrap().is_retryable());
    }
    #[tokio::test]
    async fn requests_of_a_connector_that_gave_up_are_skipped() {
        let broker = TunnelBroker::default();
        drop(broker.ask());
        let live = broker.ask();
        broker.answer(Ok(stream()));
        assert!(live.await.unwrap().is_ok());
    }
}
