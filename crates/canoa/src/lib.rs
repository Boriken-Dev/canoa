//! Protocol-neutral service selection and deny-by-default client permissions.
//! No transport, service discovery, or authentication is performed by this crate.

use std::collections::{BTreeMap, BTreeSet};

/// An identifier assigned to a service by a trusted registry.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ServiceId(pub String);

/// A discovered or manually configured service. Discovery alone does not imply trust.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Service {
    pub id: ServiceId,
    pub name: String,
    pub protocol: String,
    pub endpoint: String,
    pub verified: bool,
}

/// Identifies a caller as established by the host integration, not a self-asserted claim.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ClientId(pub String);

/// A capability that the client is allowed to exercise on an individual service.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Capability(pub String);

/// In-memory registry and explicit client grants.
#[derive(Default)]
pub struct Registry {
    services: BTreeMap<ServiceId, Service>,
    grants: BTreeSet<(ClientId, ServiceId, Capability)>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, service: Service) {
        self.services.insert(service.id.clone(), service);
    }

    pub fn get(&self, id: &ServiceId) -> Option<&Service> {
        self.services.get(id)
    }

    pub fn services(&self) -> impl Iterator<Item = &Service> {
        self.services.values()
    }

    /// Record an authorization decision made by the trusted host UI/policy layer.
    pub fn grant(&mut self, client: ClientId, service: ServiceId, capability: Capability) {
        self.grants.insert((client, service, capability));
    }

    pub fn revoke(&mut self, client: &ClientId, service: &ServiceId, capability: &Capability) {
        self.grants
            .remove(&(client.clone(), service.clone(), capability.clone()));
    }

    /// Return the selected service only when it is verified and explicitly authorized.
    pub fn resolve(
        &self,
        client: &ClientId,
        service: &ServiceId,
        capability: &Capability,
    ) -> Option<&Service> {
        let candidate = self.services.get(service)?;
        if !candidate.verified {
            return None;
        }
        self.grants
            .contains(&(client.clone(), service.clone(), capability.clone()))
            .then_some(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_and_grants_do_not_bypass_verification() {
        let mut registry = Registry::new();
        let service = ServiceId("agent-1".into());
        let client = ClientId("app.example".into());
        let capability = Capability("session.prompt".into());
        registry.register(Service {
            id: service.clone(),
            name: "Example".into(),
            protocol: "acp".into(),
            endpoint: "tcp://127.0.0.1:8765".into(),
            verified: false,
        });
        assert!(registry.resolve(&client, &service, &capability).is_none());
        registry.grant(client.clone(), service.clone(), capability.clone());
        assert!(registry.resolve(&client, &service, &capability).is_none());
        registry.register(Service {
            id: service.clone(),
            name: "Example".into(),
            protocol: "acp".into(),
            endpoint: "tcp://127.0.0.1:8765".into(),
            verified: true,
        });
        assert!(registry.resolve(&client, &service, &capability).is_some());
        registry.revoke(&client, &service, &capability);
        assert!(registry.resolve(&client, &service, &capability).is_none());
    }

    #[test]
    fn a_grant_is_scoped_to_client_service_and_capability() {
        let mut registry = Registry::new();
        let service = ServiceId("one".into());
        registry.register(Service {
            id: service.clone(),
            name: "One".into(),
            protocol: "acp".into(),
            endpoint: "stdio://agent".into(),
            verified: true,
        });
        registry.grant(
            ClientId("first".into()),
            service.clone(),
            Capability("prompt".into()),
        );
        assert!(
            registry
                .resolve(
                    &ClientId("second".into()),
                    &service,
                    &Capability("prompt".into())
                )
                .is_none()
        );
        assert!(
            registry
                .resolve(
                    &ClientId("first".into()),
                    &service,
                    &Capability("execute".into())
                )
                .is_none()
        );
    }
}
