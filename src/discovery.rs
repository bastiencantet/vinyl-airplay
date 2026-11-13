use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Serialize)]
pub struct Receiver {
    pub service: String,
    pub hostname: String,
    pub port: u16,
    pub addresses: Vec<String>,
    pub txt: BTreeMap<String, String>,
}

pub fn discover(duration: Duration) -> Result<Vec<Receiver>> {
    let daemon = ServiceDaemon::new()?;
    let mut found = BTreeMap::new();
    let events = daemon.browse("_airplay._tcp.local.")?;
    let raop = match daemon.browse("_raop._tcp.local.") {
        Ok(events) => events,
        Err(error) => {
            let _ = daemon.shutdown();
            return Err(error.into());
        }
    };
    let end = Instant::now() + duration;
    while Instant::now() < end {
        for event in events.try_iter().chain(raop.try_iter()) {
            if let ServiceEvent::ServiceResolved(info) = event {
                let mut addresses: Vec<_> = info
                    .get_addresses()
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                addresses.sort();
                found.insert(
                    info.get_fullname().to_string(),
                    Receiver {
                        service: info.get_fullname().to_string(),
                        hostname: info.get_hostname().to_string(),
                        port: info.get_port(),
                        addresses,
                        txt: info
                            .get_properties()
                            .iter()
                            .map(|p| (p.key().to_string(), p.val_str().to_string()))
                            .collect(),
                    },
                );
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    daemon.stop_browse("_airplay._tcp.local.")?;
    daemon.stop_browse("_raop._tcp.local.")?;
    let _ = daemon.shutdown()?;
    Ok(found.into_values().collect())
}
