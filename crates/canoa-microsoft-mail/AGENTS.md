# `canoa-microsoft-mail`

`canoa-microsoft-mail` implements Canoa's `mail_backend` kind (`canoa::mail_backend`) for Microsoft mailboxes:

- `ews`: Exchange Server via Exchange Web Services SOAP over `canoa::network::Network::current()`, supporting Basic sign-in, Windows logon (Negotiate/NTLM), and Autodiscover restricted to the mailbox domain or configured autodiscover host.
- `graph`: Microsoft 365 / Exchange Online via Microsoft Graph REST v1.0, using Windows Web Account Manager (`wam.ps1`) for bearer tokens.
- `outlook`: Classic Outlook for Windows via its COM object model helper (`outlook.ps1`).

`library` exports all three `mail_backend` providers for `canoa-host`'s `bundled::INCLUDED`.

`#![deny(unsafe_code)]` at the crate root; on Windows test builds (`#[cfg(all(test, windows))]`), `src/sspi_server.rs` carries the single `#[allow(unsafe_code)]` module to drive Windows' SSPI security package against the loopback exchange stand-in.
