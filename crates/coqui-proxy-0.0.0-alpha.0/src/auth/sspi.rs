//! Negotiate and NTLM through the Windows SSPI.

use alloc::string::String;
use alloc::vec::Vec;
use core::ptr;

use windows_sys::Win32::Foundation::{SEC_E_OK, SEC_I_CONTINUE_NEEDED};
use windows_sys::Win32::Security::Authentication::Identity::{
    AcquireCredentialsHandleW, DeleteSecurityContext, FreeContextBuffer, FreeCredentialsHandle,
    ISC_REQ_ALLOCATE_MEMORY, ISC_REQ_CONNECTION, InitializeSecurityContextW, SECBUFFER_TOKEN,
    SECBUFFER_VERSION, SECPKG_CRED_OUTBOUND, SECURITY_NATIVE_DREP, SecBuffer, SecBufferDesc,
};
use windows_sys::Win32::Security::Credentials::SecHandle;
use windows_sys::Win32::System::Rpc::{SEC_WINNT_AUTH_IDENTITY_UNICODE, SEC_WINNT_AUTH_IDENTITY_W};

use super::{Client, Scheme, Step};
use crate::error::{Error, Reason};
use crate::proxy::Credentials;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

fn failure(operation: &str, status: i32) -> Error {
    Error::Login(Reason::new(alloc::format!(
        "SSPI {operation} failed with status {status:#010x}"
    )))
}

/// One client side of an SSPI handshake with the `Negotiate` or `NTLM` package.
pub(crate) struct SspiClient {
    credential: SecHandle,
    context: Option<SecHandle>,
    target: Vec<u16>,
    complete: bool,
}

impl core::fmt::Debug for SspiClient {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SspiClient")
            .field("complete", &self.complete)
            .finish_non_exhaustive()
    }
}

impl SspiClient {
    /// A handshake with the service `HTTP/<host>`, as the logon session or as `credentials`.
    pub(crate) fn new(
        scheme: Scheme,
        host: &str,
        credentials: Option<&Credentials>,
    ) -> Result<Self, Error> {
        let package = wide(scheme.header_name());
        let target = wide(&alloc::format!("HTTP/{host}"));
        // The identity's strings only have to live for the call: SSPI copies them.
        let (user, domain, password);
        let mut identity: SEC_WINNT_AUTH_IDENTITY_W;
        let identity_pointer = match credentials {
            Some(credentials) => {
                let (account_domain, account_user) = match credentials.username().split_once('\\') {
                    Some((domain, user)) => (String::from(domain), String::from(user)),
                    None => (String::new(), String::from(credentials.username())),
                };
                user = account_user.encode_utf16().collect::<Vec<u16>>();
                domain = account_domain.encode_utf16().collect::<Vec<u16>>();
                password = credentials.password().encode_utf16().collect::<Vec<u16>>();
                identity = SEC_WINNT_AUTH_IDENTITY_W {
                    User: user.as_ptr().cast_mut(),
                    UserLength: user.len() as u32,
                    Domain: domain.as_ptr().cast_mut(),
                    DomainLength: domain.len() as u32,
                    Password: password.as_ptr().cast_mut(),
                    PasswordLength: password.len() as u32,
                    Flags: SEC_WINNT_AUTH_IDENTITY_UNICODE,
                };
                (&raw mut identity).cast()
            }
            None => ptr::null_mut(),
        };
        let mut credential = SecHandle {
            dwLower: 0,
            dwUpper: 0,
        };
        let mut expiry = 0i64;
        // SAFETY: `package` is a NUL-terminated wide string and `identity_pointer` is null or points
        // at an initialised identity whose buffers outlive the call; `credential` and `expiry` are
        // valid output locations.
        let status = unsafe {
            AcquireCredentialsHandleW(
                ptr::null(),
                package.as_ptr(),
                SECPKG_CRED_OUTBOUND,
                ptr::null(),
                identity_pointer,
                None,
                ptr::null(),
                &mut credential,
                &mut expiry,
            )
        };
        if status != SEC_E_OK {
            return Err(failure("AcquireCredentialsHandle", status));
        }
        Ok(Self {
            credential,
            context: None,
            target,
            complete: false,
        })
    }
}

impl Client for SspiClient {
    fn step(&mut self, input: Option<&[u8]>) -> Result<Step, Error> {
        if self.complete {
            return Ok(Step { token: None });
        }
        let mut input_buffer = SecBuffer {
            cbBuffer: input.map_or(0, |bytes| bytes.len() as u32),
            BufferType: SECBUFFER_TOKEN,
            pvBuffer: input.map_or(ptr::null_mut(), |bytes| bytes.as_ptr().cast_mut().cast()),
        };
        let mut input_description = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: 1,
            pBuffers: &raw mut input_buffer,
        };
        let mut output_buffer = SecBuffer {
            cbBuffer: 0,
            BufferType: SECBUFFER_TOKEN,
            pvBuffer: ptr::null_mut(),
        };
        let mut output_description = SecBufferDesc {
            ulVersion: SECBUFFER_VERSION,
            cBuffers: 1,
            pBuffers: &raw mut output_buffer,
        };
        let mut new_context = SecHandle {
            dwLower: 0,
            dwUpper: 0,
        };
        let mut attributes = 0u32;
        let mut expiry = 0i64;
        let existing = self
            .context
            .as_mut()
            .map_or(ptr::null_mut(), |context| &raw mut *context);
        // SAFETY: the credential is a live handle; `existing` is null or the live context from the
        // previous step; the target is a NUL-terminated wide string; both descriptions point at
        // buffers that outlive the call, and the input buffer is only read.
        let status = unsafe {
            InitializeSecurityContextW(
                &raw mut self.credential,
                existing,
                self.target.as_ptr(),
                ISC_REQ_ALLOCATE_MEMORY | ISC_REQ_CONNECTION,
                0,
                SECURITY_NATIVE_DREP,
                if input.is_some() {
                    &raw mut input_description
                } else {
                    ptr::null_mut()
                },
                0,
                &mut new_context,
                &mut output_description,
                &mut attributes,
                &mut expiry,
            )
        };
        let token = if output_buffer.pvBuffer.is_null() {
            None
        } else {
            // SAFETY: SSPI allocated `cbBuffer` readable bytes at `pvBuffer` (ISC_REQ_ALLOCATE_MEMORY).
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    output_buffer.pvBuffer.cast::<u8>(),
                    output_buffer.cbBuffer as usize,
                )
            }
            .to_vec();
            // SAFETY: the buffer came from SSPI and is freed once, here.
            unsafe { FreeContextBuffer(output_buffer.pvBuffer) };
            (!bytes.is_empty()).then_some(bytes)
        };
        if self.context.is_none() && (new_context.dwLower != 0 || new_context.dwUpper != 0) {
            self.context = Some(new_context);
        }
        match status {
            SEC_E_OK => {
                self.complete = true;
                Ok(Step { token })
            }
            SEC_I_CONTINUE_NEEDED => Ok(Step { token }),
            other => Err(failure("InitializeSecurityContext", other)),
        }
    }
}

impl Drop for SspiClient {
    fn drop(&mut self) {
        if let Some(context) = self.context.as_mut() {
            // SAFETY: the context is live and deleted once, here.
            unsafe { DeleteSecurityContext(context) };
        }
        // SAFETY: the credential is live and freed once, here.
        unsafe { FreeCredentialsHandle(&self.credential) };
    }
}
