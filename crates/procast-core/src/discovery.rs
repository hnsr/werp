//! Bounded mDNS discovery. No receiver applications are launched here.
use std::{collections::BTreeMap, net::Ipv4Addr, time::Duration};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::Serialize;

use crate::{CancellationToken, ProcastError};

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
) -> Result<Vec<Device>, ProcastError> {
    let daemon = ServiceDaemon::new().map_err(|e| ProcastError::Discovery(e.to_string()))?;
    let result = async {
        let events = daemon.browse(SERVICE).map_err(|e| ProcastError::Discovery(e.to_string()))?;
        let deadline = tokio::time::sleep(duration);
        tokio::pin!(deadline);
        let mut devices = BTreeMap::new();
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Err(ProcastError::Cancelled),
                _ = &mut deadline => break,
                event = events.recv_async() => {
                    match event.map_err(|e| ProcastError::Discovery(e.to_string()))? {
                        ServiceEvent::ServiceResolved(info) => {
                            let mut addresses: Vec<_> = info.get_addresses_v4().into_iter().collect();
                            addresses.sort();
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
    let shutdown = daemon
        .shutdown()
        .map_err(|e| ProcastError::Discovery(e.to_string()))?;
    tokio::time::timeout(Duration::from_secs(2), shutdown.recv_async())
        .await
        .map_err(|_| ProcastError::Discovery("mDNS shutdown timed out".into()))?
        .map_err(|e| ProcastError::Discovery(e.to_string()))?;
    result
}

pub fn select(devices: &[Device], selector: &str) -> Result<Device, ProcastError> {
    let exact_ids: Vec<_> = devices.iter().filter(|d| d.id == selector).collect();
    let matches: Vec<_> = if exact_ids.is_empty() {
        devices.iter().filter(|d| d.name == selector).collect()
    } else {
        exact_ids
    };
    match matches.as_slice() {
        [device] => Ok((*device).clone()),
        [] => Err(ProcastError::Discovery(format!(
            "no device matches {selector:?}"
        ))),
        _ => Err(ProcastError::Discovery(format!(
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
