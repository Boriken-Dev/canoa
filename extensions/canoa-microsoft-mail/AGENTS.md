# `canoa-microsoft-mail`

`canoa-microsoft-mail` implements Canoa's `mail_backend` kind (`canoa::mail_backend`) for Microsoft mailboxes:

- `ews`: Exchange Server via Exchange Web Services SOAP over `canoa::network::Network::current()`, supporting Basic sign-in, Windows logon (Negotiate/NTLM), and Autodiscover restricted to the mailbox domain or configured autodiscover host.
- `graph`: Microsoft 365 / Exchange Online via Microsoft Graph REST v1.0, using Windows Web Account Manager (`wam.ps1`) for bearer tokens.
- `outlook`: Classic Outlook for Windows via its COM object model helper (`outlook.ps1`).

`library` exports all three `mail_backend` providers for `canoa-host`'s `bundled::INCLUDED`.

`#![deny(unsafe_code)]` at the crate root; on Windows test builds (`#[cfg(all(test, windows))]`), `src/sspi_server.rs` carries the single `#[allow(unsafe_code)]` module to drive Windows' SSPI security package against the loopback exchange stand-in.

## Transport and sign-in notes

- `auth = "windows"` is `Transport::post_negotiate`: a `Network::login_agent` with a Negotiate rule for the server's host answers the `401` (Kerberos, then NTLM from the logon session; an NTLM exchange has three legs on one connection and the body goes with the last; a fresh agent and so a fresh exchange per call). A server that offers no scheme the logon can answer is left at its `401`, which `post` reports with the schemes offered (`it accepts Basic`); a login that fails is `MailError::Auth` naming the logon. Off Windows the build has no Negotiate (`coqui-syshttp`'s `negotiate` feature is on for Windows only), so the exchange fails there with that error. The tests' stand-in completes the exchange with the system's own security package (`sspi_server.rs`, Windows test code, the one `unsafe` in the tests of this crate).

- `Tokens` holds a token until five minutes before its `exp` claim (ten minutes' worth when it has none) and asks again once after a 401. Every request has `Prefer: IdType="ImmutableId", outlook.body-content-type="text"`. A search with `text`, `from`, `to` or `subject` is one KQL `$search` (`(words) AND from:"x" AND received>=DAY AND hasattachments:true`; Graph does not combine it with `$filter`, `$orderby` or `$skip`, and gives no count), and `unread`/`flagged` are then applied to the pages returned; any other search is `$filter` (opening with `receivedDateTime ge ...`, since Graph sorts only by a property the filter names first) with `$orderby=receivedDateTime desc` and `$count=true`. Pages are followed through `@odata.nextLink` only at Graph's own origin, until the offset and limit are met. Drafts are `POST /me/messages` (text body) or `createReply`/`createReplyAll` with the text as `comment`; no `send`, `sendMail`, `reply` or `forward` is ever called. `Change::Archive` moves to the well-known `archive`, then to a folder named Archive when Graph says `ErrorFolderNotFound`.

- Not run against a real tenant: the `$search`/`$filter` limits above, the WAM scope and `wam_compat` property, and the registration's redirect URI are from Microsoft's documentation.
