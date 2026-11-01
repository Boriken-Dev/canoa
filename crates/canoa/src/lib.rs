#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

extern crate alloc;
use alloc::collections::{BTreeMap, BTreeSet};
use core::fmt;

/// An identifier assigned to a service by a trusted registry.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ServiceId(String);

impl ServiceId {
    /// Creates a new `ServiceId` from a string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        ServiceId(id.into())
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ServiceId {
    fn from(s: &str) -> Self {
        ServiceId(s.to_string())
    }
}

/// A discovered or manually configured service. Discovery alone does not imply trust.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct Service {
    id: ServiceId,
    name: String,
    protocol: String,
    endpoint: String,
    verified: bool,
}

impl Service {
    /// Creates a new `Service` with the given parameters.
    pub fn new(
        id: ServiceId,
        name: impl Into<String>,
        protocol: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Self {
        Service {
            id,
            name: name.into(),
            protocol: protocol.into(),
            endpoint: endpoint.into(),
            verified: false,
        }
    }

    /// Marks the service as verified. Only a trusted host integration may call this, after proving
    /// the service's identity: a discovered service stays unverified.
    #[must_use]
    pub fn verified(mut self) -> Self {
        self.verified = true;
        self
    }

    /// Returns the service ID.
    pub fn id(&self) -> &ServiceId {
        &self.id
    }

    /// Returns the service name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the service protocol.
    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    /// Returns the service endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Checks if the service is verified.
    pub fn is_verified(&self) -> bool {
        self.verified
    }
}

/// Identifies a caller as established by the host integration, not a self-asserted claim.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ClientId(String);

impl ClientId {
    /// Creates a new `ClientId` from a string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        ClientId(id.into())
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for ClientId {
    fn from(s: &str) -> Self {
        ClientId(s.to_string())
    }
}

/// A capability that the client is allowed to exercise on an individual service.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Capability(String);

impl Capability {
    /// Creates a new `Capability` from a string-like value.
    pub fn new(capability: impl Into<String>) -> Self {
        Capability(capability.into())
    }

    /// Returns the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for Capability {
    fn from(s: &str) -> Self {
        Capability(s.to_string())
    }
}

/// In-memory registry and explicit client grants.
#[derive(Debug, Default)]
pub struct Registry {
    services: BTreeMap<ServiceId, Service>,
    grants: BTreeSet<(ClientId, ServiceId, Capability)>,
}

impl Registry {
    /// Creates a new `Registry`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a service in the registry.
    pub fn register(&mut self, service: Service) {
        self.services.insert(service.id.clone(), service);
    }

    /// Retrieves a service by its ID.
    pub fn get(&self, id: &ServiceId) -> Option<&Service> {
        self.services.get(id)
    }

    /// Returns an iterator over all registered services.
    pub fn services(&self) -> impl Iterator<Item = &Service> {
        self.services.values()
    }

    /// Records an authorization decision made by the trusted host UI/policy layer.
    pub fn grant(&mut self, client: ClientId, service: ServiceId, capability: Capability) {
        self.grants.insert((client, service, capability));
    }

    /// Revokes a previously granted authorization.
    pub fn revoke(&mut self, client: &ClientId, service: &ServiceId, capability: &Capability) {
        self.grants
            .remove(&(client.clone(), service.clone(), capability.clone()));
    }

    /// Returns the selected service only when it is verified and explicitly authorized.
    pub fn resolve(
        &self,
        client: &ClientId,
        service: &ServiceId,
        capability: &Capability,
    ) -> Option<&Service> {
        let candidate = self.services.get(service)?;
        if !candidate.is_verified() {
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
        let service = ServiceId::new("agent-1");
        let client = ClientId::new("app.example");
        let capability = Capability::new("session.prompt");
        registry.register(Service::new(
            service.clone(),
            "Example",
            "acp",
            "tcp://127.0.0.1:8765",
        ));
        assert!(registry.resolve(&client, &service, &capability).is_none());
        registry.grant(client.clone(), service.clone(), capability.clone());
        assert!(registry.resolve(&client, &service, &capability).is_none());
        registry.register(
            Service::new(service.clone(), "Example", "acp", "tcp://127.0.0.1:8765").verified(),
        );
        assert!(registry.resolve(&client, &service, &capability).is_some());
        registry.revoke(&client, &service, &capability);
        assert!(registry.resolve(&client, &service, &capability).is_none());
    }

    #[test]
    fn a_grant_is_scoped_to_client_service_and_capability() {
        let mut registry = Registry::new();
        let service = ServiceId::new("one");
        registry.register(Service::new(service.clone(), "One", "acp", "stdio://agent").verified());
        registry.grant(
            ClientId::new("first"),
            service.clone(),
            Capability::new("prompt"),
        );
        assert!(
            registry
                .resolve(
                    &ClientId::new("second"),
                    &service,
                    &Capability::new("prompt")
                )
                .is_none()
        );
        assert!(
            registry
                .resolve(
                    &ClientId::new("first"),
                    &service,
                    &Capability::new("execute")
                )
                .is_none()
        );
    }
}
