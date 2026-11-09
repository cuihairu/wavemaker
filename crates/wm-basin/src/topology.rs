//! Service topology model (WM-0402; docs/basin.md, docs/roadmap.md
//! Phase 4 exit criteria).
//!
//! The topology declares the services of the environment under test and
//! the hosts that run them. Two consumers hang off it:
//!
//! - Preflight (docs/basin.md "Preflight": "the target topology is fully
//!   covered") — [`Topology::uncovered_services`] is the primitive the
//!   preflight item (WM-0411) drives with a coverage predicate built from
//!   the metric catalog and mapping (WM-0403/0405).
//! - Reports — the coverage demonstration the Phase 4 exit criteria ask
//!   for ("coverage of the target service topology can be demonstrated").

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Labels;

/// What a declared service does (docs/basin.md "Metric Categories"; the
/// category list is the vocabulary — Host/Process are metric domains for
/// machines, not services, so they are not service kinds).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    /// Connection-facing entry services (loginapp-like).
    Gateway,
    /// Authentication / login services.
    Login,
    /// Game simulation services (cell/base apps).
    Game,
    /// Message queue / RPC services.
    Queue,
    Database,
    Cache,
    /// Business-level services (matchmaking, rooms).
    Business,
    /// A service the catalog vocabulary does not cover yet; the payload
    /// keeps the declaration honest instead of forcing a wrong kind.
    Other(String),
}

/// A physical or virtual machine in the environment (observable with the
/// Host metric domain).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub id: String,
    #[serde(default)]
    pub labels: Labels,
}

/// One declared service of the environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    pub id: String,
    pub kind: ServiceKind,
    /// The host running this service; must reference a declared host.
    pub host: Option<String>,
    #[serde(default)]
    pub labels: Labels,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Topology {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    hosts: BTreeMap<String, Host>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    services: BTreeMap<String, Service>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TopologyError {
    #[error("duplicate host id: {0}")]
    DuplicateHost(String),
    #[error("duplicate service id: {0}")]
    DuplicateService(String),
    #[error("host id must not be empty")]
    EmptyHostId,
    #[error("service id must not be empty")]
    EmptyServiceId,
    #[error("service {0} references undeclared host {1}")]
    UnknownHostRef(String, String),
}

impl Topology {
    /// Declare a host; duplicate ids are refused (silent overwrite would
    /// hide a misdeclared environment).
    pub fn add_host(&mut self, host: Host) -> Result<(), TopologyError> {
        if host.id.trim().is_empty() {
            return Err(TopologyError::EmptyHostId);
        }
        if self.hosts.contains_key(&host.id) {
            return Err(TopologyError::DuplicateHost(host.id));
        }
        self.hosts.insert(host.id.clone(), host);
        Ok(())
    }

    /// Declare a service; duplicate ids are refused.
    pub fn add_service(&mut self, service: Service) -> Result<(), TopologyError> {
        if service.id.trim().is_empty() {
            return Err(TopologyError::EmptyServiceId);
        }
        if self.services.contains_key(&service.id) {
            return Err(TopologyError::DuplicateService(service.id));
        }
        self.services.insert(service.id.clone(), service);
        Ok(())
    }

    pub fn hosts(&self) -> impl Iterator<Item = &Host> {
        self.hosts.values()
    }

    pub fn services(&self) -> impl Iterator<Item = &Service> {
        self.services.values()
    }

    pub fn service(&self, id: &str) -> Option<&Service> {
        self.services.get(id)
    }

    /// Structural validation: every host reference resolves.
    pub fn validate(&self) -> Result<(), TopologyError> {
        for service in self.services() {
            if let Some(host) = &service.host {
                if !self.hosts.contains_key(host) {
                    return Err(TopologyError::UnknownHostRef(
                        service.id.clone(),
                        host.clone(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Ids of services the coverage predicate fails for — the Preflight
    /// "target topology is fully covered" check in primitive form. The
    /// predicate is supplied by the caller (metric mapping, WM-0405+);
    /// ordering is stable (declaration id order).
    pub fn uncovered_services(
        &self,
        mut has_coverage: impl FnMut(&Service) -> bool,
    ) -> Vec<String> {
        self.services()
            .filter(|service| !has_coverage(service))
            .map(|service| service.id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(id: &str) -> Host {
        Host { id: id.into(), labels: Labels::new() }
    }

    fn service(id: &str, kind: ServiceKind, host: Option<&str>) -> Service {
        Service {
            id: id.into(),
            kind,
            host: host.map(String::from),
            labels: Labels::new(),
        }
    }

    #[test]
    fn builds_and_serialises_a_kbengine_like_topology() {
        let mut topology = Topology::default();
        topology.add_host(host("host-a")).expect("host");
        topology
            .add_service(service("loginapp", ServiceKind::Login, Some("host-a")))
            .expect("service");
        topology
            .add_service(service("cellapp", ServiceKind::Game, Some("host-a")))
            .expect("service");

        topology.validate().expect("valid");

        let text = serde_yaml::to_string(&topology).expect("serialise");
        let back: Topology = serde_yaml::from_str(&text).expect("reparse");
        assert_eq!(back, topology);
        assert_eq!(back.services().count(), 2);
    }

    #[test]
    fn service_kinds_round_trip_snake_case() {
        for (text, kind) in [
            ("gateway", ServiceKind::Gateway),
            ("login", ServiceKind::Login),
            ("game", ServiceKind::Game),
            ("queue", ServiceKind::Queue),
            ("database", ServiceKind::Database),
            ("cache", ServiceKind::Cache),
            ("business", ServiceKind::Business),
        ] {
            let parsed: ServiceKind = serde_yaml::from_str(text).expect("parse");
            assert_eq!(parsed, kind);
            assert_eq!(
                serde_yaml::to_string(&kind).expect("serialise").trim(),
                text
            );
        }
        let other: ServiceKind =
            serde_yaml::from_str("!other mmr-service").expect("parse");
        assert_eq!(other, ServiceKind::Other("mmr-service".into()));
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let mut topology = Topology::default();
        topology.add_host(host("h1")).expect("first host");
        assert_eq!(
            topology.add_host(host("h1")),
            Err(TopologyError::DuplicateHost("h1".into()))
        );
        topology
            .add_service(service("s1", ServiceKind::Game, None))
            .expect("first service");
        assert_eq!(
            topology.add_service(service("s1", ServiceKind::Cache, None)),
            Err(TopologyError::DuplicateService("s1".into()))
        );
    }

    #[test]
    fn unknown_host_reference_is_rejected() {
        let mut topology = Topology::default();
        topology
            .add_service(service("s1", ServiceKind::Database, Some("ghost")))
            .expect("service declared");
        assert_eq!(
            topology.validate(),
            Err(TopologyError::UnknownHostRef(
                "s1".into(),
                "ghost".into()
            ))
        );
    }

    #[test]
    fn empty_ids_are_rejected() {
        let mut topology = Topology::default();
        assert_eq!(
            topology.add_host(host(" ")),
            Err(TopologyError::EmptyHostId)
        );
        assert_eq!(
            topology.add_service(service(" ", ServiceKind::Game, None)),
            Err(TopologyError::EmptyServiceId)
        );
    }

    #[test]
    fn uncovered_services_lists_only_the_gap() {
        let mut topology = Topology::default();
        topology.add_host(host("h1")).expect("host");
        topology
            .add_service(service("loginapp", ServiceKind::Login, Some("h1")))
            .expect("service");
        topology
            .add_service(service("cellapp", ServiceKind::Game, Some("h1")))
            .expect("service");
        topology
            .add_service(service("dbmgr", ServiceKind::Database, Some("h1")))
            .expect("service");

        // Coverage: everything except database services (the way the
        // metric mapping would answer).
        let gaps = topology.uncovered_services(|service| {
            service.kind != ServiceKind::Database
        });
        assert_eq!(gaps, vec!["dbmgr".to_string()]);

        let empty = Topology::default();
        assert!(empty.uncovered_services(|_| false).is_empty());
    }
}
