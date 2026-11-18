//! The server side of a Windows security-package exchange (Negotiate, NTLM), for the stand-in Exchange: the stand-in accepts what
//! the sign-in produces by running the system's own package, so the test is of the exchange and needs no domain.
//!
//! This is a test double: the host's code never reaches it.

use core::ptr;

use windows_sys::Win32::Foundation::{SEC_E_OK, SEC_I_CONTINUE_NEEDED};
use windows_sys::Win32::Security::Authentication::Identity::{
    ASC_REQ_ALLOCATE_MEMORY, AcceptSecurityContext, AcquireCredentialsHandleW,
    DeleteSecurityContext, FreeContextBuffer, FreeCredentialsHandle, QueryContextAttributesW,
    SECBUFFER_TOKEN, SECBUFFER_VERSION, SECPKG_ATTR_NAMES, SECPKG_CRED_INBOUND,
    SECURITY_NATIVE_DREP, SecBuffer, SecBufferDesc, SecPkgContext_NamesW,
};
use windows_sys::Win32::Security::Credentials::SecHandle;

/// One connection's security context.
pub(crate) struct SspiServer {
    credential: SecHandle,
    context: Option<SecHandle>,
    /// The account the exchange authenticated, once it is complete.
    pub(crate) user: Option<String>,
}

/// What one step produced.
pub(crate) struct Accepted {
    /// The token to send back (may be empty).
    pub(crate) token: Vec<u8>,
    /// Whether the exchange is complete.
    pub(crate) complete: bool,
}

const fn empty_handle() -> SecHandle {
    SecHandle {
        dwLower: 0,
        dwUpper: 0,
    }
}

impl SspiServer {
    pub(crate) fn new() -> Self {
        let package: Vec<u16> = "Negotiate".encode_utf16().chain([0]).collect();
        let mut credential = empty_handle();
        let mut expiry = 0_i64;
        // SAFETY: `package` is NUL-terminated and outlives the call; the outputs are valid locations.
        let status = unsafe {
            AcquireCredentialsHandleW(
                ptr::null(),
                package.as_ptr(),
                SECPKG_CRED_INBOUND,
                ptr::null(),
                ptr::null(),
                None,
                ptr::null(),
                &mut credential,
                &mut expiry,
            )
        };
        assert_eq!(status, SEC_E_OK, "the server credential");
        Self {
            credential,
            context: None,
            user: None,
        }
    }

    /// Takes the client's token and answers with the package's.
    ///
    /// # Errors
    ///
    /// The package's status when it refuses the token.
    pub(crate) fn accept(&mut self, input: &[u8]) -> Result<Accepted, i32> {
        let mut input_buffer = SecBuffer {
            cbBuffer: u32::try_from(input.len()).map_err(|_| -1)?,
            BufferType: SECBUFFER_TOKEN,
            pvBuffer: input.as_ptr().cast_mut().cast(),
        };
        let input_description = SecBufferDesc {
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
        let mut new_context = empty_handle();
        let (mut attributes, mut expiry) = (0_u32, 0_i64);
        let existing = self.context.as_ref().map_or(ptr::null(), ptr::from_ref);
        // SAFETY: the credential is live; `existing` is null or the live context of the previous leg; both buffer descriptions
        // point at buffers that outlive the call.
        let status = unsafe {
            AcceptSecurityContext(
                &raw const self.credential,
                existing,
                &raw const input_description,
                ASC_REQ_ALLOCATE_MEMORY,
                SECURITY_NATIVE_DREP,
                &raw mut new_context,
                &raw mut output_description,
                &raw mut attributes,
                &raw mut expiry,
            )
        };
        let mut token = Vec::new();
        if !output_buffer.pvBuffer.is_null() {
            // SAFETY: the package allocated `cbBuffer` bytes (ASC_REQ_ALLOCATE_MEMORY); they are freed once, here.
            unsafe {
                token = core::slice::from_raw_parts(
                    output_buffer.pvBuffer.cast::<u8>(),
                    output_buffer.cbBuffer as usize,
                )
                .to_vec();
                FreeContextBuffer(output_buffer.pvBuffer);
            }
        }
        if self.context.is_none() && (new_context.dwLower != 0 || new_context.dwUpper != 0) {
            self.context = Some(new_context);
        }
        match status {
            SEC_E_OK => {
                self.user = self.authenticated_user();
                Ok(Accepted {
                    token,
                    complete: true,
                })
            }
            SEC_I_CONTINUE_NEEDED => Ok(Accepted {
                token,
                complete: false,
            }),
            other => Err(other),
        }
    }

    fn authenticated_user(&self) -> Option<String> {
        let context = self.context.as_ref()?;
        let mut names = SecPkgContext_NamesW {
            sUserName: ptr::null_mut(),
        };
        // SAFETY: the context is live and `names` is the output structure of this attribute.
        let status = unsafe {
            QueryContextAttributesW(
                ptr::from_ref(context),
                SECPKG_ATTR_NAMES,
                (&raw mut names).cast(),
            )
        };
        if status != SEC_E_OK || names.sUserName.is_null() {
            return None;
        }
        // SAFETY: the package returned a NUL-terminated wide string; it is freed once, here.
        unsafe {
            let mut length = 0;
            while *names.sUserName.add(length) != 0 {
                length += 1;
            }
            let user =
                String::from_utf16_lossy(core::slice::from_raw_parts(names.sUserName, length));
            FreeContextBuffer(names.sUserName.cast());
            Some(user)
        }
    }
}

impl Drop for SspiServer {
    fn drop(&mut self) {
        if let Some(context) = self.context.as_mut() {
            // SAFETY: the context is live and deleted once, here.
            unsafe { DeleteSecurityContext(ptr::from_mut(context)) };
        }
        // SAFETY: the credential is live and freed once, here.
        unsafe { FreeCredentialsHandle(&raw const self.credential) };
    }
}
