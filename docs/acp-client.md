# ACP v2 Flutter session controller

The shared Canoa Flutter interface has a tested **ACP v2 JSON-RPC client** and an explicit pinned-key connection dialog, using the experimental WebCrypto method from coqui-peer. The default PWA and extension chat start with offline previews. A remote connection works only when the agent endpoint implements the same method and ACP v2. There is no production pairing workflow or complete agent backend yet; see the [roadmap](roadmap.md).

## Components

| File | Responsibility |
| --- | --- |
| [coqui-web](https://github.com/Boriken-Dev/coqui-web) | Prefer WebTransport and fall back to an explicitly advertised WSS endpoint on transport-opening failure |
| [`app/lib/acp/connect.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/lib/acp/connect.dart) | Verify expected peer ID, initialize ACP v2 and close a rejected link without retrying on WSS |
| [coqui-acp](https://github.com/Boriken-Dev/coqui-acp) | Correlated JSON-RPC requests and updates, cancellation, session creation and resume, version 1 and 2 |
| [`app/lib/acp/client.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/lib/acp/client.dart) | No client until the peer is proven |
| [`app/lib/session_store.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/lib/session_store.dart) | Bind a verified agent to the correct local/remote session, stream transcript fragments and show foreground state |
| [`app/test/acp_v2_test.dart`](https://github.com/Boriken-Dev/canoa/blob/main/app/test/acp_v2_test.dart) | In-memory agent tests, peer-proof rejection, transport downgrade rules and message deduplication |

## ACP v2 semantics

After independently verified peer authentication, the client sends `initialize` using protocol version 2, `capabilities: {}`, and Boriken Dev's Canoa client information. It rejects agents not advertising v2 session support.

Session creation requires an absolute workspace path and sends `session/new`. Existing remote sessions can be attached via `session/resume`, optionally asking for a full transcript with `replayFrom: { "type": "start" }`.

`session/prompt` returns the **ID of an inserted user message**. This does not mean the agent has finished the turn. The UI consumes `session/update` notifications, including `agent_message_chunk`, full `agent_message` replacement, and `state_update` with `running`, `requires_action`, or `idle`. The agent-issued message ID is used to prevent duplicate transcript entries across chunk and full-message updates.

Every `sessionUpdate` kind in the v2 schema has a place on screen. A `plan_update` of `items` replaces the plan panel, a `tool_call_content_chunk` adds one item to its tool's card, a `notice` shows in the conversation as the agent's word with its severity, and `compaction_update` and `compaction_summary_chunk` build one entry per compaction with its summary folded away. A message or thought sent without `content` patches the one with its ID and leaves its text. A replayed user message's `resource`, `resource_link`, image or audio blocks are named in a notice under it, as a message sent with attachments is.

`session/cancel` is a notification, not an RPC expecting a turn-completion response. Agent completion is still delivered as a later idle state update.

## Peer authentication boundary

A verified transport must be produced by a callback that **cryptographically proves the expected peer identity** and answers with a channel tied to that proof, since a plain channel after a proof belongs to whoever carries it. A claimed `peerId` string, TLS hostname, or an unverified server hello is never sufficient. The ordinary ACP tests inject a fake verifier for protocol testing only. The browser method is now sourced from [coqui-peer](https://github.com/Boriken-Dev/coqui-peer/tree/main/methods/webcrypto), bundled unchanged with a source-blob hash test, and driven via a transport-level staged message queue.

Canoa's implementation intentionally **does not** connect the standard UI to unverified remote endpoints. A rejected peer proof closes the link and cannot trigger WebTransport-to-WebSocket downgrade. The native Rust responder and JavaScript↔Rust handshake tests now live in coqui-peer, with Coqui TrustStore-based acceptance and revocation. Canoa also has a [paired WSS ACP echo fixture](https://github.com/Boriken-Dev/canoa/tree/main/examples/acp-echo) that exercises a real TLS socket, Coqui proof and v2 session messages. Missing production work is agent-host pairing UI, persistent keys/sessions, production agent adapters and real remote MCP tools. The echo fixture does not execute or represent an AI agent.

The client negotiates: it requests v2 and accepts v1, whichever the agent selects.

## Permission requests in the chat

A `session/request_permission` shows as a card in its conversation with the tool, what it would run or touch, and one button per
option the agent offered, under the agent's own name for it: refusals first, then kinds Canoa does not know, then what allows,
once before always. An option sent without a name is named by its kind. The answer selects that option's `optionId`. What an
`allow_always` or `reject_always` answer remembers belongs to the agent, which the card says when it offers one: Canoa keeps
nothing of it and cannot undo it. A request offering no option Canoa can read can only be cancelled, and one whose options repeat
an id is answered cancelled without being shown, since the agent could take that id for either. A conversation holds at most 64
questions waiting; the agent's further ones are answered cancelled, and the conversation says so.

**Full access** answers a session's requests without asking. It is off unless the person turns it on, for one conversation (the
header's **Full access…**, after a dialog that says what it does) or for an agent (Settings, or `[permissions] full_access` in
Canoa.cfg), whose sessions then start with it. An agent's setting reaches only sessions of the program this app runs under that
label: a peer, a hosted agent or a browser's agent never gets it by calling itself that, and is given full access per
conversation instead. A request reaches only the conversation of the connection that sent it, whatever session id the agent
chose. While it is on, a banner on the conversation and a mark on its row say so, and the
banner's **Turn off** ends it in one action, for the agent too where the agent's setting turned it on. Each time it is turned on
is logged as `weakened: full access is on ...` and said in the conversation, and each request it answers is said there too.

It answers only with an `allow_once` option. A request offering none, including one whose only allow is `allow_always`, which the
agent would keep after full access ends, is shown to the person as before. A request already waiting when it is turned on stays
for the person. Full access is the chat's answer to the agent's requests and nothing else: it never reaches the browser
extension's grants for pages, tabs or tools, which stay [their own](acp-permissions.md#independent-browser-grants).

## The changes pane

**Changes** in a conversation's header, once a tool has changed a file, lists every file the session's completed tools
changed, each once, in the order it was first changed: beside the conversation on a wide window, as a sheet on a phone. A file
changed by before-and-after text shows the change from the first text the agent showed to the last; a patch is shown as it
came, numbered from its hunks, under the file its `changes` (ACP v2) or `+++` line names. A refused, failed or cancelled tool
changed nothing and is left out. Each file has its additions and removals, and a mark of **viewed** that holds until the file
changes again; a viewed file starts folded. Choosing a line asks for a comment and adds `path:line`, the line and the comment to
the message being written, for the person to send.

All of it is what the agent sent, never read from disk, and bounded as the transcript is: a change the agent's text was cut
from says so. A line number counts in the text the agent sent, which is the whole file only when it sent the whole file.

## An agent's questions as forms

Canoa advertises ACP's form elicitation, so an agent can ask the person something (Claude's adapter does for its
`AskUserQuestion` tool). `elicitation/create` shows as a card in its conversation, built from the request's schema: text, one
choice, several choices, a number or a whole number, yes or no, each with the agent's title and description, required ones
marked. **Send answer** is offered only once the answer fits the form; **Decline** refuses to answer and **Cancel** dismisses
it. A question of a kind Canoa does not know is shown as one it cannot answer, and when it is required only Decline and Cancel
remain. Everything the agent wrote is shown as text, with nothing that could reorder or hide what surrounds it, and no link in
it is followed. A question counts as needing the person, with its own notification (`[notifications] question`). Stop, closing
the conversation, or the agent going away answers a waiting question cancelled. URL elicitation is not offered: Canoa opens no
address an agent names. `coqui_acp` reads the form and checks the answer, as `vectors/elicitation.json` in coqui-acp says.

## A new session, started by its first message

A new session is the conversation's empty view: the folder above the message box, and under it the source, Local or
Worktree, the agent's mode, model and effort, the overlays, `+` and Send ([a new session](configuration.md#a-new-session)).
Nothing starts until the first message. Then the app starts the agent, or reaches `canoa-host` for a hosted one, sends
`initialize`, then `session/new` with the folder, the profile's `systemPrompt`, `additionalDirectories`, `_meta` and MCP
servers, sets each choice made beforehand with `session/set_config_option`, and sends the message. A choice the agent does not
offer by then is said in the conversation and not sent. A start that fails leaves the session as it was, with the message.

## Mode, model and effort beside Send

The agent's `configOptions` (v1's `id`, v2's `configId`) are shown by their ACP `category`: `mode`, `model` and `thought_level`
each have a control under the message, an option without a category is placed by its id (`mode`, `model`, `effort`,
`reasoning_effort`, `thought_level`), and every other select option is behind **More**; an agent with `modes` and no options
has its modes there. A change goes to the agent with `session/set_config_option` (in v2 with `type: "id"`, as v2 requires)
and shows once the agent's answer or a `config_option_update` says so. A value the agent refuses stays as it was, and its
reason is said in the conversation. A change made while a turn runs is sent before the next prompt, and says so meanwhile.
Before an agent has started, the controls list what its source's agent offered when it last ran, kept with the session list,
else only Default. Full access is in the mode control, in red while it is on.

## What goes with a message

`+` adds files or images, a folder reference, or what the clipboard holds; Ctrl+V (⌘V on a Mac) pastes the same way, and
files dropped on the conversation are added too. An image on the clipboard becomes an image, copied files become files, and
text stays in the message unless it is longer than `[attachments] paste_as_file`, when it goes as an attachment of its own.
Each attachment shows as a chip above the message, an image with its thumbnail, with its size and a way to take it back.

An image goes as ACP `image` content when the agent's `promptCapabilities.image` says it takes images; a file goes as an
embedded `resource` when `embeddedContext` allows it, it is text and no larger than `[attachments] embed_limit`, and as a
`resource_link` to its `file:` URI otherwise; a folder always goes as a `resource_link`, which ACP has every agent take. An
image with a file behind it goes as a link to an agent that takes no images. One that cannot go at all, such as a pasted
image for an agent without images, is refused when it is added, and a chip says how its attachment will go when that is not
as it is: nothing is left out of a message without saying so.

## Selecting and copying

The conversation is one selection, across messages, tool cards, code and diffs; the plan and the changes pane are selections
of their own. Ctrl+A (⌘A) in the conversation selects all of it and Ctrl+C copies it, as text in reading order: an agent's
Markdown as the text it shows, a code block as its code, a folded thought or summary as its heading. The context menu has
Copy and Select all. Each message and each code block has a copy button. In the message being written, Ctrl+A selects only
its text, and the text field keeps its own Cut, Copy and Paste.

## A queue while the agent works

A message written while a turn runs (Enter, or **Queue** beside Stop) waits in a queue under the conversation, with what goes
with it, instead of reaching the agent mid-turn: ACP has no way to add to a running turn. Each queued message can be edited or
removed. When a turn ends as it should, the first is sent, and the next waits for that turn to end. When a turn is stopped or
fails, the queue is held, not dropped, and says why: **Send next** goes on with it and **Clear queue** removes it. A
conversation holds at most 20 queued messages.

## Keyboard and the command palette

Ctrl (⌘ on a Mac) with: **K** the command palette, **N** a new session's view, **1** to **9** that session in the list, **Tab** and
**Shift+Tab** the next and previous session, **L** the message being written, **.** Stop, **Shift+A** and **Shift+R** the
agent's own allow-once and refuse-once options for the question shown (never an "always" one), **Shift+D** the changes,
**A** and **C** select all and copy, **V** pastes into the message, and **/** a sheet of all of these. Up in an empty message brings back the last message sent in that conversation. The palette lists
what the app can do, the other sessions to go to and the agent's own commands; typing narrows it, and Enter runs the first
left. A command chosen there is written into the message, not sent.

## Verification

```sh
cd app
flutter analyze
flutter test test/acp_v2_test.dart
flutter test
```

References: [ACP v2 initialization](https://agentclientprotocol.com/protocol/v2/initialization), [session setup](https://agentclientprotocol.com/protocol/v2/session-setup) and [v2 migration](https://agentclientprotocol.com/protocol/v2/migration).

## Message handoff

`coqui_web` owns the bounded queue of complete messages between connection setup and application use. Its `StagedChannel` holds early messages. Neither authenticates the peer or interprets the WebCrypto proof. The authenticated connection is handed to ACP **only after** Coqui succeeds.
