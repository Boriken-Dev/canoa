//! Negotiate and NTLM through GSSAPI (MIT Kerberos, Heimdal, `gss-ntlmssp`).

use alloc::vec::Vec;
use core::time::Duration;

use libgssapi::context::{ClientCtx, CtxFlags};
use libgssapi::credential::{Cred, CredUsage};
use libgssapi::name::Name;
use libgssapi::oid::{GSS_MECH_SPNEGO, GSS_NT_HOSTBASED_SERVICE, GSS_NT_USER_NAME, Oid};

use super::{Client, Scheme, Step};
use crate::error::{Error, Reason};
use crate::proxy::Credentials;

/// `1.3.6.1.4.1.311.2.2.10`, Microsoft's NTLM mechanism, which `gss-ntlmssp` provides.
const GSS_MECH_NTLM: Oid<'static> =
    Oid::from_slice(&[0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x02, 0x02, 0x0a]);

fn failure(operation: &str, error: &libgssapi::error::Error) -> Error {
    Error::Login(Reason::new(alloc::format!(
        "GSSAPI {operation} failed: {error}"
    )))
}

/// One client side of a GSSAPI handshake with the service `HTTP@<host>`.
pub(crate) struct GssapiClient {
    context: ClientCtx,
    complete: bool,
}

impl core::fmt::Debug for GssapiClient {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("GssapiClient")
            .field("complete", &self.complete)
            .finish_non_exhaustive()
    }
}

impl GssapiClient {
    /// A handshake as the Kerberos credential cache's principal, or as `credentials`.
    pub(crate) fn new(
        scheme: Scheme,
        host: &str,
        credentials: Option<&Credentials>,
    ) -> Result<Self, Error> {
        let target = Name::new(
            alloc::format!("HTTP@{host}").as_bytes(),
            Some(GSS_NT_HOSTBASED_SERVICE),
        )
        .map_err(|error| failure("naming the service", &error))?;
        let cred = match credentials {
            Some(credentials) => {
                let user = Name::new(credentials.username().as_bytes(), Some(GSS_NT_USER_NAME))
                    .map_err(|error| failure("naming the user", &error))?;
                Some(
                    Cred::acquire_with_password(
                        Some(&user),
                        credentials.password(),
                        Some(Duration::from_secs(3600)),
                        CredUsage::Initiate,
                        None,
                    )
                    .map_err(|error| failure("acquiring credentials", &error))?,
                )
            }
            None => None,
        };
        let mechanism = match scheme {
            Scheme::Negotiate => GSS_MECH_SPNEGO,
            Scheme::Ntlm => GSS_MECH_NTLM,
        };
        Ok(Self {
            context: ClientCtx::new(cred, target, CtxFlags::empty(), Some(mechanism)),
            complete: false,
        })
    }
}

impl Client for GssapiClient {
    fn step(&mut self, input: Option<&[u8]>) -> Result<Step, Error> {
        match self
            .context
            .step(input, None)
            .map_err(|error| failure("the handshake", &error))?
        {
            Some(token) => {
                let bytes: Vec<u8> = token.to_vec();
                Ok(Step {
                    token: (!bytes.is_empty()).then_some(bytes),
                })
            }
            None => {
                self.complete = true;
                Ok(Step { token: None })
            }
        }
    }
}
