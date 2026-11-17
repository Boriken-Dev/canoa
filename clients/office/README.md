# Canoa for Office

A task pane for Outlook, Word, Excel and PowerPoint that shows Canoa's chat alone (no workspace, terminal, Store or Settings)
and, first, the sessions that belong to what is open: the mail conversation in Outlook, the document in the other three.
The relation is kept with each session in the app's own list and is never sent to an agent; an agent sees a message or
document only when the person attaches it.

Nothing here is published to a marketplace. You render a manifest for your own host's address and sideload it.

## What is here

| Path | What |
| --- | --- |
| `outlook/manifest.xml.in` | The Outlook manifest (`MailApp`): a Canoa button on a message being read or written, pinnable, `ReadItem` permission, Mailbox 1.5 |
| `documents/manifest.xml.in` | The Word, Excel and PowerPoint manifest (`TaskPaneApp`), `ReadDocument` permission |
| `render.mjs` | Fills a manifest in for your origin: `node render.mjs outlook --origin https://canoa.example --out outlook.xml` (`documents` takes `--hosts Document,Workbook,Presentation`, all three by default) |
| `taskpane/` | The pages the host serves: `taskpane.html` and `taskpane.js` (read the open item through Office.js, frame the chat), `relation.js` (the relation of an item, with no Office.js in it), `dialog.html` and `dialog.js` (the Office dialog that carries a sign-in result back as a string) |
| `test/` | `npm test` runs `node --test` over `relation.js` and `render.mjs` |

The pages are compiled into `canoa-host` and answered under `/office/` only when canoa.cfg says so; the manifests are not
served by it. The app they frame is the web app the host already serves, under `/office/app/`.

## Turn it on

The host has to be reachable over HTTPS at a name the browser trusts (Office on the web loads no other page):

```toml
[host.web]
listen = "0.0.0.0:8443"
cert = "web.crt"
key = "web.key"
hosts = ["canoa.example"]
office = true
# office_frames = ["https://*.officeapps.live.com"]   # more origins that may frame the pages, once you have seen which
```

`office` is off by default and every route but the add-in's keeps its headers: see "The Office add-in's routes" in
`crates/canoa-host/AGENTS.md` for exactly what is relaxed where. The web app's page connects to the host's entries, which
listen on loopback, so a browser on another machine needs a proxy that carries them as well.

## Sideload it in Outlook on the web

1. Render the manifest: `node clients/office/render.mjs outlook --origin https://canoa.example --out outlook.xml`.
2. Check the page answers: `https://canoa.example/office/taskpane.html` loads (blank apart from a bar), and
   `https://canoa.example/office/icon-32.png` shows the icon.
3. In Outlook on the web open `https://aka.ms/olksideload`, My add-ins, Custom Add-ins, Add a custom add-in, Add from File,
   and choose `outlook.xml`. The sideload needs a Microsoft 365 or Exchange Online mailbox; whether a personal Outlook.com
   mailbox accepts a custom add-in is not confirmed.
4. Open a message and choose Canoa on its toolbar (or in the overflow menu). Pin the pane to keep it open while you move
   between messages: the chat reloads for the new conversation.
5. The first connection of the pane's key is turned away and waits on the host: `canoa peers` lists it, `canoa accept <id>`
   pairs it. A key made in an ordinary browser tab is not the pane's: storage in a frame is partitioned by the top-level
   site, so the pane has a key of its own.

Word, Excel and PowerPoint on the web: render `documents`, open a document, Home, Add-ins, More Settings, Upload My Add-in.
Desktop Office and the other Outlook clients are in Microsoft's sideloading pages
(`learn.microsoft.com/office/dev/add-ins/testing/`); an Outlook add-in sideloaded on the web also appears in the desktop
clients, where classic Outlook on Windows may take up to a day.

## How the relation is made

`relation.js` turns what Office.js gave the pane into a kind and a key:

- Mail: `conversation`, the key a hash (128 bits of SHA-256) of `item.conversationId`, so every message of a conversation,
  sent or received, gets the same one. A new message has no conversation id until it has a subject and is saved, and
  has no relation until then; a reply whose subject is changed gets a new id and so a new relation. The subject is the
  label the sessions list shows and is kept only in the app's own list.
- Document: `document`, the key a hash of the Office application and `Office.context.document.url` without its fragment
  (its query is kept, so two openings that differ in one are not related: the safe way to be wrong). A document not saved yet
  has no address and no relation. The file name in the address is the label.

The pane puts the relation in the address of the frame (`/office/app/?mode=chat&relation=<kind>:<key>&label=`), which is
how the app reads it at load. Moving between messages in a pinned pane therefore reloads the chat, and the open
conversation is not kept across it.

## Sign-in carried through the Office dialog

A sign-in page cannot be framed, so the pane's Sign in button opens `/office/dialog.html?start=/` in an Office dialog, a
separate browser instance. A sign-in page that ends by loading `/office/dialog.html#done=<string>` sends that string to the pane
through `messageParent` (strings only) and the dialog closes; the pane then reloads the chat. No host's sign-in ends that way
today: the portal's cookie is `SameSite=Strict` and its server does not serve the add-in's routes, so a portal-fronted
host does not work in a task pane yet.

## Not confirmed

None of this has run in an Office client. Microsoft's pages describe the behaviours; these points they do not settle, and
the code does not guess them:

1. Which origins frame a task pane in each client. The list in the host is a start (Outlook on the web's own addresses and
   two wildcards); check the browser console's blocked-frame report in each client and add with `office_frames`.
2. Whether Office.js needs anything beyond its one origin under this policy (`script-src` and `connect-src` name it alone);
   a violation shows in the console of the pane.
3. Whether one `TaskPaneApp` manifest may name Word, Excel and PowerPoint together; if Office refuses it, render one host
   at a time with `--hosts`.
4. Whether Office accepts the one icon (192 pixels, scaled) where the manifest asks for 16, 32, 64 and 80.
5. Whether a personal Outlook.com mailbox accepts a custom add-in from a file.
6. WebTransport, WebSocket and the non-extractable X25519 key in each client's web view (WKWebView, WebView2, Android
   System WebView); Microsoft states only which engine each uses.
7. Whether the old loopback exemption error (`We can't open this add-in from localhost`) still occurs with WebView2.
8. How the chat-only app shows a key the host has not accepted: it has no Hosts section, so the PIN pairing of the full app is
   not reachable in the pane, and the host-side `canoa accept` is the route.
9. Mobile Outlook (a mobile form factor in the manifest) and iPad are not covered.
10. Whether the `Cross-Origin-Resource-Policy: same-origin` the pane and the app keep is accepted by every client that frames
    them.
