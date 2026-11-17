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
| `taskpane/` | The pages the host serves: `taskpane.html` and `taskpane.js` (read the open item through Office.js, frame the chat, carry the sign-in), `relation.js` (the relation of an item and the messages the pane and the app exchange, with no Office.js in it), `dialog.html` and `dialog.js` (the Office dialog that carries a sign-in result back as a string) |
| `test/` | `npm test` runs `node --test` over `relation.js` and `render.mjs` |

The pages are compiled into `canoa-host` and answered under `/office/` only when the configuration says so; the manifests are not
served by it. The app they frame is the web app the host already serves, under `/office/app/`. Both the standalone host
(`canoa serve --web`) and the portal (`canoa portal`) serve them.

## Turn it on

The host has to be reachable over HTTPS at a name the browser trusts (Office on the web loads no other page).

A host of your own (`canoa serve --web`), in canoa.cfg:

```toml
[host.web]
listen = "0.0.0.0:8443"
cert = "web.crt"
key = "web.key"
hosts = ["canoa.example"]
office = true
# office_frames = ["https://*.officeapps.live.com"]   # more origins that may frame the pages, once you have seen which
```

The portal (`canoa portal`, each person signing in with their account on the machine), in its settings file:

```toml
hosts = ["canoa.example"]
office = true
# office_frames = ["https://*.officeapps.live.com"]
# office_handoff_seconds = 60                          # how long a finished sign-in waits for the pane to claim it
```

`office` is off by default and every route but the add-in's keeps its headers: see "The Office add-in's routes" in
`crates/canoa-host/AGENTS.md` for exactly what is relaxed where. A proxy in front of either must not add `X-Frame-Options` or
its own `Content-Security-Policy` to `/office/` paths. The web app's page connects to the host's entries, which listen on
loopback, so a browser on another machine needs a proxy that carries them as well (the portal relays them itself).

Render the manifests for your address once (Node 20 or later, no packages to install):

```sh
node clients/office/render.mjs outlook   --origin https://canoa.example --out outlook.xml
node clients/office/render.mjs documents --origin https://canoa.example --out documents.xml
```

Check the host answers before you sideload: `https://canoa.example/office/taskpane.html` loads (a bar and a Sign in button on a
portal), and `https://canoa.example/office/icon-32.png` shows the icon.

## Sideload it

Which account each client needs is part of each step. None needs a store, an Azure registration or an administrator, except
where a step says so.

### Outlook on the web

Account: a **Microsoft 365 or Exchange Online mailbox** (work, school or a Microsoft 365 plan with Exchange Online). Whether a
personal Outlook.com mailbox accepts a custom add-in from a file is not confirmed. If your organisation has turned off custom
add-ins for users, the option below is missing: an administrator uploads `outlook.xml` instead (Microsoft 365 admin center,
Settings, Integrated apps, Add-ins, Deploy Add-in, upload the file; it can take up to 72 hours to appear).

1. Open `https://aka.ms/olksideload` in the browser where you are signed in to Outlook on the web. It opens **Add-Ins for Outlook**.
2. Under **Custom Add-ins** choose **Add a custom add-in**, **Add from File**, and pick `outlook.xml`. Accept the warning that
   the add-in is not from the store. There is no "Add from URL": use the file.
3. Open a message. Choose **Canoa** on its toolbar (or in the **...** menu). The pane opens on the right.
4. Pin the pane (the pin at its top) to keep it open while you move between messages: the sessions listed first follow the
   message's conversation, with no reload.
5. First use: see "The first time in the pane", below.

An add-in sideloaded on the web also appears in new Outlook and classic Outlook on Windows and in Outlook on Mac for the same
mailbox; classic Outlook on Windows may take up to a day to show it.

### Word and Excel on the web (and PowerPoint)

Account: a **Microsoft account** (a document in OneDrive) or a **Microsoft 365 work or school account** (a document in OneDrive or
SharePoint). Use the same account the document is open under.

1. Render `documents.xml` (above).
2. In the browser, open or create a document in Word on the web (or a workbook in Excel on the web).
3. **Home**, **Add-ins**, **More Settings**, then **Upload My Add-in**. Choose **Browse**, pick `documents.xml`, **Upload**.
4. Canoa is on the ribbon (Home), or under Add-ins. Choose it: the pane opens beside the document, with the sessions of this
   document first.
5. First use: see "The first time in the pane", below.

Office keeps the uploaded manifest in that browser's local storage: clearing site data, a private window or another browser
means uploading it again.

### Word and Excel on Windows (and PowerPoint), through a shared-folder catalog

Account: any Office on Windows that runs add-ins (Microsoft 365 apps, retail Office, Office LTSC 2021 or 2024), signed in with
whatever account you use; the add-in itself needs none. This route is for testing: Microsoft does not support it for
production deployment.

1. Make a folder for the catalog, for example `C:\OfficeAddins`, and copy `documents.xml` into it.
2. Share it: right-click the folder, **Properties**, **Sharing**, **Share**, and add yourself with **Read** permission. Note the
   network path it shows, `\\YOUR-COMPUTER\OfficeAddins`.
3. In Word or Excel: **File**, **Options**, **Trust Center**, **Trust Center Settings**, **Trusted Add-in Catalogs**. In
   **Catalog URL** type that network path, choose **Add catalog**, tick **Show in Menu**, **OK**, **OK**.
4. Close every Office window and open Word or Excel again (the catalog is read at start).
5. Open a document. **Home**, **Add-ins**, **Advanced**, the **SHARED FOLDER** tab, choose **Canoa**, **Add**. The pane opens
   beside the document. (Older builds have it under **Insert**, **My Add-ins**.)
6. First use: see "The first time in the pane", below.

An add-in from a shared folder is listed under **My Add-ins**, **SHARED FOLDER** each time; it does not show on the ribbon.
Windows hosts the pane in WebView2, which is installed with Office; the pane's address is HTTPS, so no certificate step is needed
for a host behind a public certificate. A host behind a certificate your computer does not trust needs that certificate trusted
on the computer first.

### Outlook on Windows and on the Mac

Sideload through Outlook on the web (above): the add-in is registered on the mailbox and shows up in the desktop clients. Word,
Excel and PowerPoint on the Mac read a manifest copied to `~/Library/Containers/com.microsoft.<App>/Data/Documents/wef`
(`Word`, `Excel`, `Powerpoint`). Mobile Outlook takes an add-in only through an administrator's deployment and is not covered.

## The first time in the pane

**Behind a portal** (`canoa portal`, for example `https://canoa.example`): the pane opens with a **Sign in** button.

1. Choose **Sign in**. Office opens a small window with the portal's sign-in page: sign in as you would in a browser, by
   password (from inside the network) or by certificate. Any second step the portal asks for is asked here.
2. The window closes by itself and the chat appears with the sessions of what is open first. You are signed in for the portal's
   session time (eight hours by default); after that, **Sign in** again.
3. Nothing else is paired: the portal introduces the pane's own key to your host.

**A host of your own** (`canoa serve --web`): the chat opens, and the host turns the pane's key away until you pair it. The pane
then shows a card **Pair this pane with its host**:

1. On the host's computer run the command the card shows, `canoa pair --method noise --origin https://canoa.example` (the
   origin is the pane's).
2. It prints an address (`ws://...`) and a six-digit PIN. Type both into the card and choose **Pair**. A PIN is good for one try.
3. The pane starts over, connected. `canoa accept <id>` after `canoa peers` is the other way: it pairs the key the pane showed.

A key made in an ordinary browser tab is not the pane's: storage in a frame is partitioned by the top-level site, so each Office
client (and each browser) has a key of its own and pairs once.

## How the relation is made

`relation.js` turns what Office.js gave the pane into a kind and a key:

- Mail: `conversation`, the key a hash (128 bits of SHA-256) of `item.conversationId`, so every message of a conversation,
  sent or received, gets the same one. A new message has no conversation id until it has a subject and is saved, and
  has no relation until then; a reply whose subject is changed gets a new id and so a new relation. The subject is the
  label the sessions list shows and is kept only in the app's own list.
- Document: `document`, the key a hash of the Office application and `Office.context.document.url` without its fragment
  (its query is kept, so two openings that differ in one are not related: the safe way to be wrong). A document not saved yet
  has no address and no relation. The file name in the address is the label.

The pane puts the first relation in the address of the frame (`/office/app/?mode=chat&relation=<kind>:<key>&label=`), which is
how the app reads it at load. After that it tells the app through a message (`{"canoa":"relation","relation":{...}}`, a string
posted to the frame, which accepts it only from the page that frames it and of its own origin). The app says `ready` once it
listens, and again if it starts over. So in a pinned Outlook pane moving between messages switches the list of related sessions,
and the open conversation stays where it is. The pane offers nothing about the open session itself.

## Sign-in carried through the Office dialog

Behind a portal the chat is the portal's site, and its session cookie is `SameSite=Strict`: a browser does not send it from
inside Outlook's page, which is another site. A sign-in page cannot be framed either. So:

1. The pane asks `GET /office/session` (it answers JSON, `401` when the frame has no session; a host with no portal answers its own
   page, and the chat is shown as it is).
2. **Sign in** opens `/office/dialog.html?start=/office/signin` in an Office dialog, a separate browser instance. The portal
   marks that browser as signing in for the dialog and sends it to its sign-in page, which ends at
   `/office/dialog.html#done=<string>` instead of the site. The dialog sends the string to the pane with `messageParent` and
   closes. The string names a sign-in that waits in the portal's memory for a minute and is good for one claim; no session was
   made in the dialog.
3. The pane posts the string to `POST /office/session` from its own frame. The portal opens a session for it, as a sign-in at the
   site would have, and sets the frame's own cookie, `SameSite=None; Secure; HttpOnly; Partitioned`: a third-party frame can
   carry it, and a browser that blocks third-party cookies keeps one for this top-level site alone.
4. The chat is loaded.

A host that does not serve a portal never goes through this.

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
8. Whether `Partitioned` cookies are accepted by every client's web view. Chromium (Edge, Chrome, WebView2) does; Firefox
   partitions third-party cookies by itself; Safari and WKWebView (Outlook and Office on the Mac, iOS) block third-party cookies
   and are not expected to keep the pane's session. If a client signs in and the pane asks again at once, look in its console
   for the cookie being rejected.
9. Whether the dialog's link to the pane survives the sign-in pages in every client: the portal sends no
   `Cross-Origin-Opener-Policy` while a browser signs in for the dialog, since Office blocks a message from a dialog that sends
   `same-origin` (error 12006). Check `messageParent` arrives in each client.
10. Mobile Outlook (a mobile form factor in the manifest) and iPad are not covered.
11. Whether the `Cross-Origin-Resource-Policy: same-origin` the pane and the app keep is accepted by every client that frames
    them.
12. Whether Office's own page lets a frame of another site post a message to its parent in every client (the app's `ready` and
    `signin` messages), and whether the dialog opens from a click inside the frame in each browser's popup rules.
