// The provider's own window: what a person decides about a page and an agent, and where they add and remove agents.
const main = document.querySelector("#main");
const params = new URLSearchParams(location.search);
const askId = params.get("ask");

const send = (message) => chrome.runtime.sendMessage(message);

function element(tag, properties = {}, ...children) {
  const node = Object.assign(document.createElement(tag), properties);
  node.append(...children);
  return node;
}

function validAgent(label, url, peer) {
  if (!label.trim()) return "Give it a name.";
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    return "That is not a URL.";
  }
  const local = ["localhost", "127.0.0.1", "[::1]"].includes(parsed.hostname);
  if (parsed.protocol !== "wss:" && !(parsed.protocol === "ws:" && local)) return "Use wss://, or ws:// on this machine.";
  if (peer !== "" && !/^sha256:[0-9a-f]{64}$/.test(peer)) return "The peer id looks like sha256: and 64 hex digits. Discovery is not trust: get it from the agent's owner.";
  return null;
}

function agentForm(onAdd) {
  const label = element("input", { type: "text", placeholder: "Name" });
  const url = element("input", { type: "text", placeholder: "wss://host/path or ws://127.0.0.1:8765" });
  const peer = element("input", { type: "text", placeholder: "sha256:… (empty: nothing proves who answers)" });
  const status = element("p", { className: "warn" });
  const add = element("button", { type: "button", className: "quiet", textContent: "Add this agent" });
  add.addEventListener("click", async () => {
    const problem = validAgent(label.value, url.value.trim(), peer.value.trim());
    status.textContent = problem ?? "";
    if (problem !== null) return;
    await onAdd({ label: label.value.trim(), url: url.value.trim(), peer: peer.value.trim() });
    label.value = url.value = peer.value = "";
  });
  return element("fieldset", {}, element("legend", { textContent: "Add one by hand" }),
    element("label", { textContent: "Name" }), label, element("label", { textContent: "Address" }), url,
    element("label", { textContent: "Peer id" }), peer, add, status);
}

/** Searching is the person's choice, each time: it starts a program and listens on the network. */
function searchButton(onFound) {
  const status = element("p");
  const search = element("button", { type: "button", className: "quiet", textContent: "Look for agents nearby" });
  search.addEventListener("click", async () => {
    search.disabled = true;
    status.textContent = "Looking…";
    const { candidates, noCompanion } = await send({ acp: "nearby" });
    search.disabled = false;
    status.textContent = noCompanion
      ? "That needs coqui-acp-bridge installed: coqui-acp-bridge companion install --extension-id " + chrome.runtime.id
      : candidates.length === 0 ? "Nothing found." : "Anyone can claim anything here. Check a key with the agent's owner before you choose it.";
    onFound(candidates);
  });
  return element("div", {}, search, status);
}

const WHERE = { "dns-sd": "says it is on this network", local: "on this machine", manual: "added by hand" };

function grantView(details) {
  const added = [];
  const picks = element("div");
  const row = (agent, checked, isNew) => {
    const box = element("input", { type: "checkbox", checked });
    box.dataset.key = agent.key ?? "";
    box.dataset.fresh = isNew ? String(added.length - 1) : "";
    const about = agent.proven ? "proven" : agent.peer ? "not proven yet" : "unverified";
    const lines = [element("strong", { textContent: agent.label }), ` — ${WHERE[agent.source] ?? WHERE.manual}, ${about}`];
    if (agent.url) lines.push(element("br"), element("code", { textContent: agent.url }));
    lines.push(element("br"), element("code", { textContent: agent.peer ?? "no peer id: nothing proves who answers" }));
    if (agent.sameKeyAs) lines.push(element("br"), `Names the same key as ${agent.sameKeyAs}. Choosing this moves it here.`);
    picks.append(element("label", { className: "pick" }, box, element("span", {}, ...lines)));
  };
  for (const agent of details.agents) row(agent, details.peers.includes(agent.peer), false);
  if (details.agents.length === 0) picks.append(element("p", { textContent: "No agents yet. Add one below." }));

  const cwd = element("input", { type: "text", placeholder: "An absolute path on the agent's machine" });
  const lifetime = element("select", {},
    element("option", { value: "once", textContent: "Until this tab closes" }),
    element("option", { value: "always", textContent: "Until I revoke it" }));
  const approve = element("input", { type: "checkbox" });
  const allow = element("button", { type: "button", textContent: "Allow" });
  const refuse = element("button", { type: "button", className: "quiet", textContent: "Refuse" });
  allow.addEventListener("click", () => {
    const boxes = [...picks.querySelectorAll("input[type=checkbox]:checked")];
    send({
      acp: "answer",
      ask: askId,
      answer: {
        allow: boxes.filter((box) => box.dataset.key).map((box) => box.dataset.key),
        add: boxes.filter((box) => box.dataset.fresh !== "").map((box) => added[Number(box.dataset.fresh)]),
        cwd: cwd.value.trim() || null,
        lifetime: lifetime.value,
        permissions: approve.checked,
        mcpOrigins: details.mcpOrigins,
      },
    });
  });
  refuse.addEventListener("click", () => send({ acp: "answer", ask: askId, answer: null }));

  const asked = [
    element("p", {}, element("span", { className: "site", textContent: details.origin }), " asks to use your agents."),
  ];
  if (details.purpose) asked.push(element("p", { className: "theirs", textContent: `In its own words: ${details.purpose}` }));
  asked.push(element("p", { textContent: "It will be able to start conversations and read the answers. It will see the peer id of each agent you choose. It sees nothing about the others." }));
  if (details.mcpOrigins.length > 0) {
    asked.push(element("p", { className: "warn", textContent: `It may also connect them to: ${details.mcpOrigins.join(", ")}` }));
  }
  main.replaceChildren(...asked, element("h2", { textContent: "Choose agents" }), picks,
    searchButton((candidates) => {
      for (const candidate of candidates) {
        if (added.some((agent) => agent.url === candidate.url && agent.peer === candidate.peer)) continue;
        added.push(candidate);
        // Never ticked for the person: choosing one is what accepts its key.
        row(candidate, false, true);
      }
    }),
    agentForm(async (agent) => {
      added.push(agent);
      row(agent, true, true);
    }),
    element("label", { textContent: "Working directory for its sessions" }), cwd,
    element("label", { textContent: "For how long" }), lifetime,
    element("label", { className: "pick" }, approve,
      element("span", { className: "warn", textContent: "This site can approve what the agent does for me" })),
    allow, refuse);
}

function permissionView(details) {
  const { toolCall, options } = details.request.params ?? {};
  const title = toolCall?.title ?? toolCall?.kind ?? "a tool";
  const buttons = (Array.isArray(options) ? options : []).map((option) => {
    const button = element("button", { type: "button", textContent: String(option.name ?? option.optionId) });
    if (String(option.kind ?? "").startsWith("reject")) button.className = "quiet";
    button.addEventListener("click", () => send({ acp: "answer", ask: askId, answer: option.optionId }));
    return button;
  });
  const cancel = element("button", { type: "button", className: "quiet", textContent: "Cancel" });
  cancel.addEventListener("click", () => send({ acp: "answer", ask: askId, answer: null }));
  main.replaceChildren(
    element("p", {}, element("strong", { textContent: details.label }), " wants to use a tool, in a conversation started by ",
      element("span", { className: "site", textContent: details.origin }), "."),
    element("p", { textContent: String(title) }), ...buttons, cancel);
}

async function manageView() {
  const { agents, grants } = await send({ acp: "manage" });
  const agentRows = agents.map((agent) => {
    const remove = element("button", { type: "button", className: "quiet", textContent: "Remove" });
    remove.addEventListener("click", async () => {
      await send({ acp: "remove", key: agent.key });
      manageView();
    });
    return element("li", {}, element("strong", { textContent: agent.label }), " ", element("code", { textContent: `${agent.url} ${agent.peer ?? "no peer id"}` }), element("br"), remove);
  });
  const grantRows = grants.map((grant) => {
    const label = agents.find((agent) => agent.key === grant.agent)?.label ?? grant.agent;
    const revoke = element("button", { type: "button", className: "quiet", textContent: "Revoke" });
    revoke.addEventListener("click", async () => {
      await send({ acp: "revoke", grant });
      manageView();
    });
    const extra = grant.permissions ? ", can approve for me" : "";
    return element("li", {}, element("span", { className: "site", textContent: grant.origin }), ` may use ${label} (${grant.lifetime}${extra})`, element("br"), revoke);
  });
  const found = element("ul");
  main.replaceChildren(element("h1", { textContent: "Canoa agents" }), element("ul", {}, ...agentRows),
    searchButton((candidates) => {
      found.replaceChildren(...candidates.map((candidate) => {
        const add = element("button", { type: "button", className: "quiet", textContent: "Add" });
        add.addEventListener("click", async () => {
          await send({ acp: "add", agent: candidate });
          manageView();
        });
        return element("li", {}, element("strong", { textContent: candidate.label }), ` — ${WHERE[candidate.source]}`, element("br"),
          element("code", { textContent: `${candidate.url} ${candidate.peer ?? "no peer id"}` }), element("br"), add);
      }));
    }), found,
    agentForm(async (agent) => {
      await send({ acp: "add", agent });
      manageView();
    }),
    element("h2", { textContent: "Sites" }),
    grantRows.length > 0 ? element("ul", {}, ...grantRows) : element("p", { textContent: "No site has been allowed." }));
}

if (params.has("manage")) {
  manageView();
} else {
  const details = await send({ acp: "details", ask: askId });
  if (details === null) main.textContent = "This request has ended.";
  else if (details.kind === "permission") permissionView(details);
  else grantView(details);
}
