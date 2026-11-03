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

/** What another party wrote, without what could reorder or hide what is shown around it. */
const clean = (value, limit = 300) => String(value ?? "").replace(/\p{Cf}|\p{Cc}/gu, " ").trim().slice(0, limit);

/**
 * Nothing here answers for the first moment it can be seen, or after it comes to the front: a page can time a window to land under
 * a click meant for something else. Buttons added later are the person's own doing.
 */
function holdClicks() {
  let timer;
  const hold = () => {
    const buttons = [...main.querySelectorAll("button")];
    for (const button of buttons) button.disabled = true;
    clearTimeout(timer);
    timer = setTimeout(() => {
      for (const button of buttons) button.disabled = false;
    }, 500);
  };
  window.addEventListener("focus", hold);
  hold();
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

/** A row about an agent. A record naming a key the person knows is about that agent, so its own choice of name is not shown. */
function describeAgent(agent) {
  const about = agent.proven ? "proven" : agent.peer ? "not proven yet" : "unverified";
  const lines = agent.sameKeyAs
    ? [element("strong", { textContent: agent.sameKeyAs }), " — something on this network says it is now at this address"]
    : [element("strong", { textContent: agent.label }), ` — ${WHERE[agent.source] ?? WHERE.manual}, ${about}`];
  if (agent.url) lines.push(element("br"), element("code", { textContent: agent.url }));
  lines.push(element("br"), element("code", { textContent: agent.peer ?? "no peer id: nothing proves who answers" }));
  if (agent.offers?.length > 0) {
    lines.push(element("br"), `Says it serves: ${agent.offers.join(", ")}`);
  }
  if (agent.sameKeyAs) {
    lines.push(element("br"), element("span", { className: "warn", textContent: `Anyone can name a public key. Choosing this moves ${agent.sameKeyAs} here, and whoever is asking gets ${agent.sameKeyAs}. If it cannot prove the key there, it goes back.` }));
  }
  if (agent.sameNameAs) {
    lines.push(element("br"), element("span", { className: "warn", textContent: `You have another agent called ${agent.sameNameAs}, with a different key.` }));
  }
  return lines;
}

function grantView(details) {
  const added = [];
  const picks = element("div");
  const row = (agent, checked, isNew) => {
    const box = element("input", { type: "checkbox", checked });
    box.dataset.key = agent.key ?? "";
    box.dataset.fresh = isNew ? String(added.length - 1) : "";
    picks.append(element("label", { className: "pick" }, box, element("span", {}, ...describeAgent(agent))));
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

  const asked = details.own
    ? [element("p", { textContent: "Canoa's chat asks to use your agents. What they ask to do is answered in the conversation." })]
    : [element("p", {}, element("span", { className: "site", textContent: details.origin }), " asks to use your agents.")];
  if (details.purpose) asked.push(element("p", { className: "theirs", textContent: `In its own words: ${details.purpose}` }));
  asked.push(element("p", { textContent: "It will be able to start conversations and read the answers. It will see the peer id of each agent you choose. It sees nothing about the others." }));
  if (details.mcpOrigins.length > 0) {
    asked.push(element("p", { className: "warn", textContent: `It may also connect them to: ${details.mcpOrigins.join(", ")}` }));
  }
  // Above the list, so a row that appears cannot land under a second click.
  main.replaceChildren(...asked, element("h2", { textContent: "Choose agents" }),
    searchButton((candidates) => {
      for (const candidate of candidates) {
        if (added.some((agent) => agent.url === candidate.url && agent.peer === candidate.peer)) continue;
        added.push(candidate);
        // Never ticked for the person: choosing one is what accepts its key.
        row(candidate, false, true);
      }
    }), picks,
    agentForm(async (agent) => {
      added.push(agent);
      row(agent, true, true);
    }),
    element("label", { textContent: "Working directory for its sessions" }), cwd,
    element("label", { textContent: "For how long" }), lifetime,
    ...(details.own ? [] : [element("label", { className: "pick" }, approve,
      element("span", { className: "warn", textContent: "This site can approve what the agent does for me" }))]),
    allow, refuse);
  holdClicks();
}

function permissionView(details) {
  const { toolCall, options } = details.request.params ?? {};
  const title = clean(details.request.params?.title ?? toolCall?.title ?? toolCall?.kind ?? "a tool");
  const buttons = (Array.isArray(options) ? options : []).slice(0, 8).map((option) => {
    const button = element("button", { type: "button", textContent: clean(option.name ?? option.optionId, 80) });
    if (String(option.kind ?? "").startsWith("reject")) button.className = "quiet";
    button.addEventListener("click", () => send({ acp: "answer", ask: askId, answer: option.optionId }));
    return button;
  });
  const cancel = element("button", { type: "button", className: "quiet", textContent: "Cancel" });
  cancel.addEventListener("click", () => send({ acp: "answer", ask: askId, answer: null }));
  main.replaceChildren(
    element("p", {}, element("strong", { textContent: details.label }), " wants to use a tool, in a conversation started by ",
      element("span", { className: "site", textContent: details.origin }), "."),
    element("p", { textContent: title }), ...buttons, cancel);
  holdClicks();
}

const WHAT = {
  "browser.read": "read a tab: its text, and what can be clicked in it",
  "browser.interact": "click and type in a tab",
  "webmcp.invoke": "run the tools a page offers to agents",
  "mcp.discover": "see which MCP servers a page names",
  "browser.debug": "control a tab with the debugger",
};

function browserView(details) {
  const tabs = element("div");
  const pick = (tab) => {
    const box = element("input", { type: "radio", name: "tab", value: String(tab.id) });
    box.dataset.origin = tab.origin;
    tabs.append(element("label", { className: "pick" }, box,
      element("span", {}, element("span", { className: "site", textContent: tab.origin }), element("br"), tab.title ?? "")));
  };
  // The tab in front first, and none chosen: reaching a page is what the person is deciding.
  for (const tab of [...details.tabs].sort((a, b) => Number(b.active) - Number(a.active))) pick(tab);
  if (details.tabs.length === 0) tabs.append(element("p", { textContent: "No tab here can be shared." }));
  const minutes = element("select", {}, ...details.minutes.map((value) => element("option", { value: String(value), textContent: `For ${value} minutes` })));
  const status = element("p", { className: "warn" });
  const allow = element("button", { type: "button", textContent: "Allow this tab" });
  const refuse = element("button", { type: "button", className: "quiet", textContent: "Refuse" });
  allow.addEventListener("click", async () => {
    const chosen = tabs.querySelector("input:checked");
    if (chosen === null) {
      status.textContent = "Choose a tab.";
      return;
    }
    // Chrome's own prompt, if this site has none yet. It needs this click.
    const reached = await chrome.permissions.request({ origins: [`${chosen.dataset.origin}/*`] });
    if (!reached) {
      status.textContent = "Without that, the agent could not reach this tab.";
      return;
    }
    send({ acp: "answer", ask: askId, answer: { tabId: Number(chosen.value), minutes: Number(minutes.value) } });
  });
  refuse.addEventListener("click", () => send({ acp: "answer", ask: askId, answer: null }));
  const who = details.via === "program"
    ? [element("p", {}, element("strong", { textContent: `A program on ${details.label}` }), ` asks to ${WHAT[details.capability] ?? details.capability}.`),
      element("p", { className: "warn", textContent: "Canoa cannot tell which program: whatever runs as you on that machine can ask here, and shares what you allow." }),
      element("p", {}, element("code", { textContent: details.peer }))]
    : [element("p", {}, element("strong", { textContent: details.label }), ` asks to ${WHAT[details.capability] ?? details.capability}.`)];
  main.replaceChildren(
    ...who,
    element("p", { className: "theirs", textContent: details.reason ? `In its own words: ${details.reason}` : "It gave no reason." }),
    ...(details.capability === "browser.debug" ? [element("p", { className: "warn", textContent: "The debugger can read this site's cookies and anything else on the page, and act as you there." })] : []),
    element("p", { textContent: "What it reads there can try to talk it into things. Share only what you would show it yourself." }),
    tabs, minutes, allow, refuse, status);
  holdClicks();
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
        const add = element("button", { type: "button", className: "quiet", textContent: candidate.sameKeyAs ? `Move ${candidate.sameKeyAs} here` : "Add" });
        add.addEventListener("click", async () => {
          await send({ acp: "add", agent: candidate });
          manageView();
        });
        return element("li", {}, ...describeAgent(candidate), element("br"), add);
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
  else if (details.kind === "browser") browserView(details);
  else grantView(details);
}
