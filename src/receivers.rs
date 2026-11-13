//! Live Bonjour catalog on the capture host. Only discovered LAN addresses may be selected.
use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    net::Ipv4Addr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receiver {
    pub id: String,
    pub name: String,
    pub address: String,
    pub model: String,
    #[serde(default)]
    pub enclosure_color: Option<u8>,
    pub available: bool,
    pub last_seen: u64,
    pub service: String,
}
#[derive(Clone)]
pub struct Catalog(Arc<Mutex<BTreeMap<String, Receiver>>>);
impl Catalog {
    pub fn start(path: PathBuf) -> Result<Self> {
        let mut cached: Vec<Receiver> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        cached.retain(|r| valid_id(&r.id) && valid_address(&r.address));
        cached.truncate(128);
        for r in &mut cached {
            r.available = false;
        }
        let catalog = Self(Arc::new(Mutex::new(
            cached.into_iter().map(|r| (r.id.clone(), r)).collect(),
        )));
        let daemon = ServiceDaemon::new()?;
        let events = daemon.browse("_airplay._tcp.local.")?;
        let target = catalog.clone();
        tokio::spawn(async move {
            loop {
                let mut changed = false;
                for event in events.try_iter() {
                    let mut entries = target.0.lock().expect("receiver catalog");
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let Some(ip) = info
                                .get_addresses()
                                .iter()
                                .filter_map(|a| a.to_string().parse::<Ipv4Addr>().ok())
                                .filter(|a| a.is_private() || a.is_link_local())
                                .min_by_key(|ip| (ip.is_link_local(), *ip))
                            else {
                                continue;
                            };
                            if info.get_port() == 0 {
                                continue;
                            }
                            let service = info.get_fullname().to_string();
                            let mac = info
                                .get_property_val_str("deviceid")
                                .unwrap_or("")
                                .chars()
                                .filter(|c| c.is_ascii_hexdigit())
                                .collect::<String>()
                                .to_lowercase();
                            let id = if mac.len() == 12 {
                                format!("airplay:{mac}")
                            } else {
                                format!("airplay:{:x}", Sha256::digest(service.as_bytes()))
                            };
                            let name = service
                                .strip_suffix("._airplay._tcp.local.")
                                .unwrap_or(&service)
                                .to_string();
                            if entries.len() >= 128 && !entries.contains_key(&id) {
                                continue;
                            }
                            entries.insert(
                                id.clone(),
                                Receiver {
                                    id,
                                    name,
                                    address: format!("{ip}:{}", info.get_port()),
                                    model: info
                                        .get_property_val_str("model")
                                        .unwrap_or("AirPlay")
                                        .to_string(),
                                    enclosure_color: enclosure_color(
                                        info.get_property_val_str("model").unwrap_or(""),
                                        info.get_property_val_str("c"),
                                    ),
                                    available: true,
                                    last_seen: SystemTime::now()
                                        .duration_since(UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs(),
                                    service,
                                },
                            );
                            changed = true;
                        }
                        ServiceEvent::ServiceRemoved(_, service) => {
                            for r in entries.values_mut().filter(|r| r.service == service) {
                                r.available = false;
                                changed = true;
                            }
                        }
                        _ => {}
                    }
                }
                if changed && let Ok(bytes) = serde_json::to_vec(&target.list()) {
                    let temp = path.with_extension("tmp");
                    if std::fs::write(&temp, bytes).is_ok() {
                        let _ = std::fs::rename(temp, &path);
                    }
                }
                let _ = &daemon; // Retain the daemon; it handles DNS TTL expiry/removal.
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        });
        Ok(catalog)
    }
    pub fn list(&self) -> Vec<Receiver> {
        self.0
            .lock()
            .expect("receiver catalog")
            .values()
            .cloned()
            .collect()
    }
    pub fn find(&self, id: &str) -> Option<Receiver> {
        self.list().into_iter().find(|r| {
            r.id == id || (matches!(id, "salon" | "chambre") && r.name.eq_ignore_ascii_case(id))
        })
    }
    pub fn resolve(&self, id: &str) -> Result<String> {
        self.find(id)
            .map(|r| r.address)
            .with_context(|| format!("Appareil AirPlay non détecté : {id}."))
    }
}
// HomePod TXT `c` matches /info.deviceEnclosureColor on the tested receivers.
// Restrict this interpretation to Apple speakers; other vendors may reuse `c`.
fn enclosure_color(model: &str, value: Option<&str>) -> Option<u8> {
    if !model.starts_with("AudioAccessory") {
        return None;
    }
    value.and_then(|v| v.parse::<u8>().ok()).filter(|v| *v > 0)
}
pub fn valid_id(id: &str) -> bool {
    id.strip_prefix("airplay:")
        .is_some_and(|v| matches!(v.len(), 12 | 64) && v.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn valid_address(address: &str) -> bool {
    address
        .parse::<std::net::SocketAddrV4>()
        .is_ok_and(|a| a.port() != 0 && (a.ip().is_private() || a.ip().is_link_local()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enclosure_colors_are_optional_and_vendor_scoped() {
        assert_eq!(enclosure_color("AudioAccessory1,1", Some("2")), Some(2));
        assert_eq!(enclosure_color("AudioAccessory5,1", Some("9")), Some(9));
        assert_eq!(enclosure_color("C655", Some("9")), None);
        for value in [None, Some(""), Some("0"), Some("256"), Some("../9")] {
            assert_eq!(enclosure_color("AudioAccessory5,1", value), None);
        }
        let old = r#"{"id":"airplay:001122334455","name":"Salon","address":"192.168.50.20:7000","model":"AudioAccessory1,1","available":false,"last_seen":0,"service":"Salon._airplay._tcp.local."}"#;
        assert!(
            serde_json::from_str::<Receiver>(old)
                .unwrap()
                .enclosure_color
                .is_none()
        );
    }
    #[test]
    fn identifiers_and_addresses_are_not_free_form_destinations() {
        assert!(valid_id("airplay:001122334455"));
        for id in [
            "airplay:localhost",
            "192.168.50.20:7000",
            "airplay:001122334455;echo",
        ] {
            assert!(!valid_id(id));
        }
        assert!(valid_address("192.168.1.42:7000"));
        for address in ["127.0.0.1:7000", "1.1.1.1:80", "192.168.1.42:0"] {
            assert!(!valid_address(address));
        }
    }
}
