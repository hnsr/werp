//! Bounded mDNS discovery. No receiver applications are launched here.
use std::{
    collections::{BTreeMap, HashSet},
    net::Ipv4Addr,
    time::Duration,
};

use mdns_sd::{HostnameResolutionEvent, Receiver, ServiceDaemon, ServiceEvent};
use serde::Serialize;
use tokio::task::JoinSet;

use crate::{CancellationToken, WerpError};

const SERVICE: &str = "_googlecast._tcp.local.";

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: String,
    pub addresses: Vec<Ipv4Addr>,
    pub port: u16,
    pub capabilities: Option<u32>,
}

/// IPv4 discovery. Unknown capabilities are reported without guessing support.
pub async fn discover(
    duration: Duration,
    cancel: &CancellationToken,
) -> Result<Vec<Device>, WerpError> {
    let daemon = ServiceDaemon::new().map_err(|e| WerpError::Discovery(e.to_string()))?;
    let result = async {
        let events = daemon
            .browse(SERVICE)
            .map_err(|e| WerpError::Discovery(e.to_string()))?;
        collect(events, duration, cancel, |host| {
            daemon
                .resolve_hostname(host, None)
                .map_err(|e| WerpError::Discovery(e.to_string()))
        })
        .await
    }
    .await;
    let shutdown = daemon
        .shutdown()
        .map_err(|e| WerpError::Discovery(e.to_string()))?;
    tokio::time::timeout(Duration::from_secs(2), shutdown.recv_async())
        .await
        .map_err(|_| WerpError::Discovery("mDNS shutdown timed out".into()))?
        .map_err(|e| WerpError::Discovery(e.to_string()))?;
    result
}

async fn collect(
    events: Receiver<ServiceEvent>,
    duration: Duration,
    cancel: &CancellationToken,
    mut resolve: impl FnMut(&str) -> Result<Receiver<HostnameResolutionEvent>, WerpError>,
) -> Result<Vec<Device>, WerpError> {
    let mut lookups = JoinSet::new();
    let result = async {
        let deadline = tokio::time::sleep(duration);
        tokio::pin!(deadline);
        let mut devices = BTreeMap::new();
        let mut queried = HashSet::new();
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(WerpError::Cancelled),
                _ = &mut deadline => break,
                Some(_) = lookups.join_next(), if !lookups.is_empty() => {},
                event = events.recv_async() => {
                    match event.map_err(|e| WerpError::Discovery(e.to_string()))? {
                        ServiceEvent::ServiceResolved(info) => {
                            let mut addresses: Vec<_> = info.get_addresses_v4().into_iter().collect();
                            addresses.sort();
                            tracing::debug!(ipv4_count = addresses.len(), total_address_count = info.get_addresses().len(), "mDNS service resolved");
                            // mdns-sd accepts an AAAA-only service as resolved, so
                            // browse may never ask for A records. Explicitly query
                            // its hostname; new A records also emit ServiceResolved.
                            // Query once per host during this bounded scan.
                            if addresses.is_empty() && queried.insert(info.get_hostname().to_ascii_lowercase()) {
                                let replies = resolve(info.get_hostname())?;
                                tracing::debug!("querying hostname because no IPv4 address was resolved");
                                lookups.spawn(async move {
                                    // Drain the bounded reply channel so it cannot
                                    // block the daemon. Browse owns the snapshots.
                                    while replies.recv_async().await.is_ok() {}
                                });
                            }
                            let device = Device {
                                id: info.get_property_val_str("id").unwrap_or(info.get_fullname()).into(),
                                name: info.get_property_val_str("fn").unwrap_or(info.get_fullname()).into(),
                                model: info.get_property_val_str("md").unwrap_or("unknown").into(),
                                addresses, port: info.get_port(),
                                capabilities: info.get_property_val_str("ca").and_then(|s| s.parse().ok()),
                            };
                            devices.insert(info.get_fullname().to_owned(), device);
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => { devices.remove(&fullname); }
                        _ => {}
                    }
                }
            }
        }
        Ok(devices.into_values().collect())
    }.await;
    // The overall scan deadline/cancellation applies to hostname lookups too.
    // The caller shuts down the daemon, stopping its remaining DNS queries.
    lookups.shutdown().await;
    result
}

pub fn select(devices: &[Device], selector: &str) -> Result<Device, WerpError> {
    let exact_ids: Vec<_> = devices.iter().filter(|d| d.id == selector).collect();
    let matches: Vec<_> = if exact_ids.is_empty() {
        devices.iter().filter(|d| d.name == selector).collect()
    } else {
        exact_ids
    };
    match matches.as_slice() {
        [device] => Ok((*device).clone()),
        [] => Err(WerpError::Discovery(format!(
            "no device matches {selector:?}"
        ))),
        _ => Err(WerpError::Discovery(format!(
            "ambiguous name {selector:?}; select an ID: {}",
            matches
                .iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdns_sd::ServiceInfo;

    fn resolved(addresses: &[&str]) -> ServiceEvent {
        ServiceEvent::ServiceResolved(Box::new(
            ServiceInfo::new(SERVICE, "Test TV", "test-tv.local.", addresses, 8009, None)
                .unwrap()
                .as_resolved_service(),
        ))
    }

    #[tokio::test]
    async fn ipv6_only_resolution_requests_ipv4_and_accepts_updated_snapshot() {
        let (events_tx, events) = flume::unbounded();
        events_tx.send(resolved(&["2001:db8::1"])).unwrap();
        events_tx.send(resolved(&["2001:db8::1"])).unwrap();
        let (replies_tx, replies) = flume::bounded(1);
        let mut queried = Vec::new();
        let devices = collect(
            events,
            Duration::from_millis(30),
            &CancellationToken::new(),
            |host| {
                queried.push(host.to_owned());
                // A lookup's A answer updates the shared mDNS cache and generates
                // another service snapshot, independent of hostname reply draining.
                events_tx
                    .send(resolved(&["2001:db8::1", "192.0.2.10"]))
                    .unwrap();
                Ok(replies.clone())
            },
        )
        .await
        .unwrap();
        assert_eq!(queried, vec!["test-tv.local."]);
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices[0].addresses,
            vec!["192.0.2.10".parse::<Ipv4Addr>().unwrap()]
        );
        // No worker is left consuming the hostname channel after the scan.
        drop(replies);
        assert!(replies_tx.is_disconnected());
    }

    #[tokio::test]
    async fn already_resolved_ipv4_needs_no_additional_lookup() {
        let (sender, events) = flume::unbounded();
        sender.send(resolved(&["192.0.2.10"])).unwrap();
        let devices = collect(
            events,
            Duration::from_millis(10),
            &CancellationToken::new(),
            |_| panic!("IPv4 already available"),
        )
        .await
        .unwrap();
        assert_eq!(devices[0].addresses.len(), 1);
    }

    #[tokio::test]
    async fn unanswered_ipv4_lookup_obeys_scan_deadline_and_cancellation() {
        for cancelled in [false, true] {
            let (sender, events) = flume::unbounded();
            sender.send(resolved(&["2001:db8::1"])).unwrap();
            let (replies_tx, replies) = flume::bounded(1);
            let token = CancellationToken::new();
            let result = tokio::time::timeout(
                Duration::from_secs(1),
                collect(
                    events,
                    if cancelled {
                        Duration::from_secs(60)
                    } else {
                        Duration::from_millis(20)
                    },
                    &token,
                    |_| {
                        if cancelled {
                            token.cancel();
                        }
                        Ok(replies.clone())
                    },
                ),
            )
            .await
            .expect("hostname lookup extended scan or blocked cancellation");
            if cancelled {
                assert!(matches!(result, Err(WerpError::Cancelled)));
            } else {
                assert!(result.unwrap()[0].addresses.is_empty());
            }
            drop(replies);
            assert!(replies_tx.is_disconnected());
        }
    }

    #[test]
    fn duplicate_names_require_an_id_and_ids_take_precedence() {
        let a = Device {
            id: "one".into(),
            name: "TV".into(),
            model: "test".into(),
            addresses: vec![],
            port: 8009,
            capabilities: None,
        };
        let b = Device {
            id: "two".into(),
            ..a.clone()
        };
        assert!(select(&[a.clone(), b.clone()], "TV").is_err());
        assert_eq!(select(&[a.clone(), b.clone()], "two").unwrap().id, "two");
        let c = Device {
            name: "one".into(),
            ..b
        };
        assert_eq!(select(&[a, c], "one").unwrap().id, "one");
    }
}
