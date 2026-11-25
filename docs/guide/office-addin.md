# The Office add-in

The add-in puts {{ name }}'s chat in a task pane beside a mail in Outlook, or a document in Word, Excel or PowerPoint. The pane shows the chat alone (no workspace, terminal, Store or Settings) and, first, the sessions that belong to what is open: the mail conversation, or the document.

![The pane's session list beside an open mail conversation](../assets/screens/office-pane-dark.png#only-dark)
![The pane's session list beside an open mail conversation](../assets/screens/office-pane-light.png#only-light)

The picture is the pane's own page, drawn with sample data outside Office: sessions tied to a conversation called "Quarterly numbers", each with **Resume**, and **New session** for another one on the same conversation.

What belongs to the item is a note kept with each session in {{ name }}'s own list. It is never sent to an agent: an agent sees a message or a document only when you attach it.

## Set it up

Nothing is published to a marketplace. You turn the add-in on in your own host and sideload a manifest made for its address.

1. Serve the pane over HTTPS at a name the browser trusts. In {{ config_file }}:

    ```toml
    [host.web]
    listen = "0.0.0.0:8443"
    cert = "web.crt"
    key = "web.key"
    hosts = ["canoa.example"]
    office = true
    ```

    For classic desktop Office on the same computer, `office_local = true` with a loopback `listen` needs no sign-in and no pairing.
2. Render the manifest for that address: `node clients/office/render.mjs outlook --origin https://canoa.example --out outlook.xml` for Outlook, or the `documents` manifest for Word, Excel and PowerPoint.
3. Sideload it: Outlook and Office on the web take the file in their add-in settings; on Windows `clients/office/sideload.ps1 add` registers the document manifest for Word, Excel and PowerPoint.

The add-in's folder in the repository, `clients/office/`, has the exact steps for each application, and [the host's page](../host.md#the-web-app) lists every setting.

## Use it

!!! example "Worked example: draft a reply to the mail you are reading"
    1. Open a mail in Outlook and choose **Canoa** in the ribbon. The pane opens and lists the sessions about this conversation, newest first.
    2. Press **New session**, or **Resume** one.
    3. Attach the message to the chat. The agent sees the message because you attached it, not because it was open.
    4. Ask for a draft. While the pane is open, the agent can use the application's own tools (the open item, the document, the workbook), and a change is asked in the pane first, with the same card as in [Permissions](permissions.md).
    5. Close the pane. The sessions stay in the list of {{ name }}, and open again beside the same conversation.

## Signing in

On a host of your own the pane connects to the host that served it. A computer several people use runs the [portal](../portal.md), where the pane signs in through a dialog Office opens: see [the web app and the portal](web-and-portal.md#the-portal).

## About the picture

The pane has not been captured inside Outlook or Word. The picture above is the pane's page drawn by {{ name }}'s own widgets with sample data.
