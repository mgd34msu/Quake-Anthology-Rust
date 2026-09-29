//! In-memory loopback hub ported from `src/network/common/loopback.ts`.
//!
//! Multiple named connection pairs preserve the rereleases' local
//! split-screen clients. Delivery is synchronous through the hub.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use super::endpoint::NetworkAddress;
use super::transport::{
    monotonic_clock, Clock, DatagramLimits, DatagramTransport, PacketQueue, ReceiveEvent, TransportError,
    UNIFIED_DATAGRAM_LIMITS,
};

/// Named loopback hub (`LoopbackHub`).
pub struct LoopbackHub {
    inner: Mutex<HubInner>,
    limits: DatagramLimits,
    now: Clock,
}

struct HubInner {
    endpoints: HashMap<String, Weak<LoopbackTransportInner>>,
}

impl LoopbackHub {
    /// Create a hub with `limits` and a millisecond clock.
    #[must_use]
    pub fn new(limits: DatagramLimits, now: Clock) -> Self {
        Self {
            inner: Mutex::new(HubInner {
                endpoints: HashMap::new(),
            }),
            limits,
            now,
        }
    }

    /// Bind a uniquely named endpoint (`bind`).
    pub fn bind(self: &Arc<Self>, id: &str) -> Result<Arc<LoopbackTransport>, TransportError> {
        if id.is_empty() {
            return Err(TransportError::Closed(
                "Loopback endpoint must have a unique nonempty name".to_owned(),
            ));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| TransportError::Closed("Loopback hub is closed".to_owned()))?;
        if inner
            .endpoints
            .get(id)
            .is_some_and(|endpoint| endpoint.upgrade().is_some())
        {
            return Err(TransportError::Closed(
                "Loopback endpoint must have a unique nonempty name".to_owned(),
            ));
        }
        let address = NetworkAddress::Loopback { id: id.to_owned() };
        let queue = PacketQueue::new(self.limits, self.now.clone())?;
        let transport = Arc::new(LoopbackTransportInner {
            address,
            queue,
            hub: Arc::downgrade(self),
            ended: AtomicBool::new(false),
        });
        inner.endpoints.insert(id.to_owned(), Arc::downgrade(&transport));
        Ok(Arc::new(LoopbackTransport { inner: transport }))
    }

    /// Close every endpoint (`close`).
    pub fn close(&self) {
        let endpoints: Vec<Weak<LoopbackTransportInner>> = self
            .inner
            .lock()
            .map(|inner| inner.endpoints.values().cloned().collect())
            .unwrap_or_default();
        for endpoint in endpoints {
            if let Some(transport) = endpoint.upgrade() {
                LoopbackTransport { inner: transport }.close();
            }
        }
    }
}

impl Default for LoopbackHub {
    fn default() -> Self {
        Self::new(UNIFIED_DATAGRAM_LIMITS, monotonic_clock())
    }
}

use std::sync::atomic::{AtomicBool, Ordering};

struct LoopbackTransportInner {
    address: NetworkAddress,
    queue: PacketQueue<NetworkAddress>,
    hub: Weak<LoopbackHub>,
    ended: AtomicBool,
}

impl LoopbackTransportInner {
    fn opened(&self) -> Result<(), TransportError> {
        if self.ended.load(Ordering::Relaxed) {
            return Err(TransportError::Closed("Loopback transport is closed".to_owned()));
        }
        Ok(())
    }
}

/// Bound loopback endpoint (`LoopbackTransport`).
pub struct LoopbackTransport {
    inner: Arc<LoopbackTransportInner>,
}

impl LoopbackTransport {
    fn opened(&self) -> Result<(), TransportError> {
        self.inner.opened()
    }
}

impl DatagramTransport for LoopbackTransport {
    type Address = NetworkAddress;

    fn address(&self) -> NetworkAddress {
        self.inner.address.clone()
    }

    fn closed(&self) -> bool {
        self.inner.ended.load(Ordering::Relaxed)
    }

    fn send(&self, to: &NetworkAddress, payload: &[u8]) -> Result<bool, TransportError> {
        self.opened()?;
        if payload.len() > self.inner.queue.limits.max_bytes {
            return Err(TransportError::Oversize);
        }
        let NetworkAddress::Loopback { id } = to else {
            return Ok(false);
        };
        let hub = self
            .inner
            .hub
            .upgrade()
            .ok_or_else(|| TransportError::Closed("Loopback transport is closed".to_owned()))?;
        let peer = hub
            .inner
            .lock()
            .ok()
            .and_then(|inner| inner.endpoints.get(id).cloned())
            .and_then(|peer| peer.upgrade());
        let Some(peer) = peer else {
            return Ok(false);
        };
        peer.opened()?;
        peer.queue.accept(self.inner.address.clone(), payload, false);
        Ok(true)
    }

    fn poll(&self) -> Result<Option<ReceiveEvent<NetworkAddress>>, TransportError> {
        self.opened()?;
        Ok(self.inner.queue.poll())
    }

    fn subscribe_readable(&self, listener: Arc<dyn Fn() + Send + Sync>) -> Result<u64, TransportError> {
        self.opened()?;
        self.inner.queue.subscribe(listener)
    }

    fn unsubscribe(&self, token: u64) {
        self.inner.queue.unsubscribe(token);
    }

    fn close(&self) {
        if self.inner.ended.swap(true, Ordering::Relaxed) {
            return;
        }
        if let NetworkAddress::Loopback { id } = &self.inner.address {
            if let Some(hub) = self.inner.hub.upgrade() {
                if let Ok(mut inner) = hub.inner.lock() {
                    inner.endpoints.remove(id);
                }
            }
        }
        self.inner.queue.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_pair_exchanges_datagrams() {
        let hub = Arc::new(LoopbackHub::default());
        let first = hub.bind("a").unwrap();
        let second = hub.bind("b").unwrap();
        assert!(hub.bind("a").is_err());
        assert!(first.send(&second.address(), &[1, 2, 3]).unwrap());
        assert!(!first
            .send(
                &NetworkAddress::Loopback {
                    id: "missing".to_owned()
                },
                &[1]
            )
            .unwrap());
        let event = second.poll().unwrap().unwrap();
        assert!(matches!(event, ReceiveEvent::Packet { .. }));
        assert!(second.poll().unwrap().is_none());
    }
}
