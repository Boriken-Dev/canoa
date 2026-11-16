// The provider's own window: what a person decides about a page and an agent, and where they add and remove agents.
import { keyAnswer } from "./answer-keys.js";
import { defaultBrowserName, readBrowserName, writeBrowserName } from "./browser-name.js";

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
let holdTimer;
let held = [];
let holdListening = false;
function holdClicks() {
  // Buttons and tick boxes alike: a click meant for something else must not tick or answer. What was already off stays off.
  for (const control of held) control.disabled = false;
  held = [...main.querySelectorAll("button, input[type=checkbox]")].filter((control) => !control.disabled);
  for (const control of held) control.disabled = true;
  clearTimeout(holdTimer);
  holdTimer = setTimeout(() => {
    for (const control of held) control.disabled = false;
    held = [];
  }, 500);
  if (!holdListening) {
    holdListening = true;
    window.addEventListener("focus", holdClicks);
  }
}

let keyHandler = null;

/** Enter accepts and Escape declines, and each button says so. Disabled buttons ignore the click, so the first moment is held too. */
function answerKeys(accept, decline) {
  if (keyHandler !== null) document.removeEventListener("keydown", keyHandler);
  keyHandler = null;
  const mark = (button, key) => button.append(" ", element("kbd", { textContent: key }));
  if (decline) mark(decline, "Esc");
  if (accept) mark(accept, "↵");
  keyHandler = (event) => {
    const answer = keyAnswer(event);
    const button = answer === "accept" ? accept : answer === "decline" ? decline : null;
    if (!button) return;
    event.preventDefault();
    button.click();
  };
  document.addEventListener("keydown", keyHandler);
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
  // The method the agent's key proves by, which its owner says with the key; never guessed from a connection.
  const method = element("select", {}, element("option", { value: "", textContent: "Choose: its owner says which" }),
    element("option", { value: "noise", textContent: "Noise" }), element("option", { value: "webcrypto", textContent: "WebCrypto" }));
  const status = element("p", { className: "warn" });
  const add = element("button", { type: "button", className: "quiet", textContent: "Add this agent" });
  add.addEventListener("click", async () => {
    const problem = validAgent(label.value, url.value.trim(), peer.value.trim())
      ?? (peer.value.trim() !== "" && method.value === "" ? "Choose the method the agent proves by: coqui-acp-bridge serve and canoa serve print it." : null);
    status.textContent = problem ?? "";
    if (problem !== null) return;
    await onAdd({ label: label.value.trim(), url: url.value.trim(), peer: peer.value.trim(), method: method.value || undefined });
    label.value = url.value = peer.value = "";
  });
  return element("fieldset", {}, element("legend", { textContent: "Add one by hand" }),
    element("label", { textContent: "Name" }), label, element("label", { textContent: "Address" }), url,
    element("label", { textContent: "Peer id" }), peer, element("label", { textContent: "Method" }), method, add, status);
}

/** Pairing by a PIN the agent's machine shows: the PIN proves that machine's key, under the Noise method. */
function pairForm(onPaired) {
  const browserName = element("input", { type: "text", maxLength: 60 });
  readBrowserName(chrome.storage.local).then((name) => {
    browserName.value = name ?? defaultBrowserName(navigator.userAgentData);
  }).catch(() => {
    browserName.value = defaultBrowserName(navigator.userAgentData);
  });
  const label = element("input", { type: "text", placeholder: "Name" });
  const url = element("input", { type: "text", placeholder: "wss://host/path or ws://127.0.0.1:8765" });
  const pairUrl = element("input", { type: "text", placeholder: "where coqui-acp-bridge pair-accept --method noise waits" });
  const pin = element("input", { type: "text", placeholder: "the six digits it shows", autocomplete: "off" });
  const status = element("p", { className: "warn" });
  const pair = element("button", { type: "button", className: "quiet", textContent: "Pair" });
  pair.addEventListener("click", async () => {
    const problem = validAgent(label.value, url.value.trim(), "") ?? validAgent(label.value, pairUrl.value.trim(), "");
    status.textContent = problem ?? "";
    if (problem !== null) return;
    // Kept on this browser: nothing in the pairing carries it to the agent's machine yet.
    await writeBrowserName(chrome.storage.local, browserName.value).catch(() => null);
    pair.disabled = true;
    status.textContent = "Pairing…";
    const answer = await send({ acp: "pair", pairing: { label: label.value.trim(), url: url.value.trim(), pairUrl: pairUrl.value.trim(), pin: pin.value.trim() } });
    pair.disabled = false;
    // A PIN is spent once tried, right or wrong: the next try needs a new one.
    pin.value = "";
    if (answer?.error) {
      status.textContent = `Not paired: ${answer.error}. Ask the machine for a new PIN.`;
      return;
    }
    status.textContent = "";
    label.value = url.value = pairUrl.value = "";
    onPaired(answer.agent);
  });
  // pair-accept refuses a browser unless it lists the browser's origin, so a page cannot spend the PIN.
  const command = element("p", {}, "On the machine: ", element("code", { textContent: `coqui-acp-bridge pair-accept --method noise --listen 127.0.0.1:PORT --origin chrome-extension://${chrome.runtime.id}` }));
  return element("fieldset", {}, element("legend", { textContent: "Pair with a code" }), command,
    element("label", { textContent: "Agent's name" }), label, element("label", { textContent: "Address" }), url,
    element("label", { textContent: "Pairing address" }), pairUrl, element("label", { textContent: "PIN" }), pin,
    element("label", { textContent: "Name this browser so you can identify it later" }), browserName, pair, status);
}

/** Searching is the person's choice, each time: it starts a program and listens on the network. */
function searchButton(onFound) {
  const status = element("p");
  const search = element("button", { type: "button", className: "quiet", textContent: "Look for agents nearby" });
  search.addEventListener("click", async () => {
    search.disabled = true;
    status.textContent = "Looking…";
    const { candidates, needsCanoa, why, refused, notes = [] } = await send({ acp: "nearby" });
    search.disabled = false;
    status.textContent = needsCanoa
      ? "That needs Canoa installed on this computer: it looks for agents for the browser."
      : refused ? "Canoa on this computer does not let this browser look for agents (it lacks the discover grant)."
      : why ? `Canoa on this computer could not look: ${why}`
      : [candidates.length === 0 ? "Nothing found." : "Anyone can claim anything here. Check a key with the agent's owner before you choose it.", ...notes].join(" ");
    onFound(candidates);
  });
  return element("div", {}, search, status);
}

const WHERE = { "dns-sd": "says it is on this network", local: "on this machine", manual: "added by hand" };

/** Whether an origin names this computer or a private network, which an agent on this computer can reach and a stranger cannot. */
function onThisComputer(origin) {
  let host;
  try {
    host = new URL(origin).hostname.toLowerCase();
  } catch {
    return false;
  }
  if (host === "localhost" || host.endsWith(".localhost") || host.endsWith(".local") || host.endsWith(".internal") || !host.includes(".") ) return true;
  if (host.startsWith("[")) return host === "[::1]" || /^\[(fc|fd|fe80)/.test(host);
  const parts = host.split(".").map(Number);
  if (parts.length !== 4 || parts.some((part) => !Number.isInteger(part))) return false;
  const [a, b] = parts;
  return a === 10 || a === 127 || a === 0 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 169 && b === 254);
}

/** What the window tells of Canoa on this computer, in the person's words; the page has no say in any of it. */
function localSaid(view, count) {
  if (view.state === "running" && view.paired) {
    return count > 0
      ? "This site can tell these agents what to do. They run on this computer as you, with your settings and tools, and can read your files. Canoa gives each conversation a folder of its own for this site, which is not a limit on what they can reach. You are asked only about what the agents' own settings ask about."
      : "Canoa on this computer serves no agents.";
  }
  if (view.state === "running") {
    return view.changed
      ? "This browser is paired with another Canoa than the one running on this computer. Pair it again in Canoa's options page."
      : "Canoa on this computer is running, but this browser is not paired with it. Pair it in Canoa's options page first.";
  }
  if (view.state === "stopped") return "Canoa on this computer is not running.";
  if (view.state === "not-installed") return "Canoa is not installed on this computer.";
  return `Canoa on this computer cannot be used right now${view.why ? `: ${view.why}` : ""}.`;
}

/** A row about an agent. A record naming a key the person knows is about that agent, so its own choice of name is not shown. */
function describeAgent(agent) {
  const about = agent.proven ? "proven" : agent.peer ? "not proven yet" : "unverified";
  const lines = agent.sameKeyAs
    ? [element("strong", { textContent: agent.sameKeyAs }), " — something on this network says it is now at this address"]
    : [element("strong", { textContent: agent.label }), ` — ${agent.local ? "Canoa on this computer" : (WHERE[agent.source] ?? WHERE.manual)}, ${about}`];
  if (agent.url) lines.push(element("br"), element("code", { textContent: agent.url }));
  lines.push(element("br"), element("code", { textContent: agent.peer ?? "no peer id: nothing proves who answers" }));
  if (agent.peer && !agent.local) lines.push(` (${agent.method ?? "webcrypto"})`);
  if (agent.unreachable) lines.push(element("br"), element("span", { className: "warn", textContent: `Unreachable from this browser: ${agent.unreachable}` }));
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
  const here = element("div");
  const choices = element("div", {}, picks, here);
  const row = (agent, checked, isNew) => {
    const box = element("input", { type: "checkbox", checked });
    box.dataset.key = agent.key ?? "";
    box.dataset.fresh = isNew ? String(added.length - 1) : "";
    if (agent.local) box.dataset.local = "yes";
    (agent.local ? here : picks).append(element("label", { className: "pick" }, box, element("span", {}, ...describeAgent(agent))));
  };
  // What a page names as agents it knows never ticks one of Canoa's: every one shares the key of the computer's Canoa.
  for (const agent of details.agents) row(agent, details.peers.includes(agent.peer) && !agent.local, false);
  if (details.agents.filter((agent) => !agent.local).length === 0) picks.append(element("p", { textContent: "No other agents yet. Add one below." }));

  const cwd = element("input", { type: "text", placeholder: "An absolute path on the agent's machine" });
  const lifetime = element("select", {},
    element("option", { value: "once", textContent: "Until this tab closes" }),
    element("option", { value: "always", textContent: "Until I revoke it" }));
  const approve = element("input", { type: "checkbox" });
  const allow = element("button", { type: "button", textContent: "Allow" });
  const refuse = element("button", { type: "button", className: "quiet", textContent: "Refuse" });
  allow.addEventListener("click", () => {
    const boxes = [...choices.querySelectorAll("input[type=checkbox]:checked")];
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
  answerKeys(allow, refuse);

  const asked = details.own
    ? [element("p", { textContent: "Canoa's chat asks to use your agents. What they ask to do is answered in the conversation." })]
    : [element("p", {}, element("span", { className: "site", textContent: details.origin }), " asks to use your agents.")];
  if (details.purpose) asked.push(element("p", { className: "theirs", textContent: `In its own words: ${details.purpose}` }));
  asked.push(element("p", { textContent: "It will be able to start conversations and read the answers. It will see the peer id of each agent you choose. It sees nothing about the others." }));
  if (details.mcpOrigins.length > 0) {
    asked.push(element("p", { className: "warn", textContent: `It may also connect them to: ${details.mcpOrigins.map((origin) => (onThisComputer(origin) ? `${origin} (this computer or a private network)` : origin)).join(", ")}` }));
  }
  const hereStatus = element("p", { textContent: "Looking for Canoa on this computer…" });
  const start = element("button", { type: "button", className: "quiet", textContent: "Start Canoa" });
  start.hidden = true;
  // What the host says now: the person may have started it, stopped it or paired since the page asked. Its rows are replaced by
  // what it serves now, keeping the ones the person ticked that are still there.
  const learnHere = ({ view, agents }) => {
    hereStatus.textContent = localSaid(view, agents.length);
    start.hidden = view.state !== "stopped";
    start.disabled = false;
    const ticked = new Set([...here.querySelectorAll("input:checked")].map((box) => box.dataset.key));
    here.replaceChildren();
    for (const agent of agents) row(agent, ticked.has(agent.key), false);
    holdClicks();
  };
  send({ acp: "host" }).then(learnHere, () => {
    hereStatus.textContent = "Canoa on this computer could not be asked.";
  });
  start.addEventListener("click", () => {
    start.disabled = true;
    hereStatus.textContent = "Starting Canoa…";
    send({ acp: "start" }).then(learnHere, () => {
      start.disabled = false;
    });
  });
  // Above the list, so a row that appears cannot land under a second click.
  main.replaceChildren(...asked, element("h2", { textContent: "Choose agents" }),
    element("h3", { textContent: "Canoa on this computer" }), hereStatus, start,
    searchButton((candidates) => {
      for (const candidate of candidates) {
        if (added.some((agent) => agent.url === candidate.url && agent.peer === candidate.peer)) continue;
        added.push(candidate);
        // Never ticked for the person: choosing one is what accepts its key.
        row(candidate, false, true);
      }
    }), choices,
    agentForm(async (agent) => {
      added.push(agent);
      row(agent, true, true);
    }),
    element("label", { textContent: "Working directory for its sessions (for agents added by hand; Canoa on this computer makes its own)" }), cwd,
    element("label", { textContent: "For how long" }), lifetime,
    ...(details.own ? [] : [element("label", { className: "pick" }, approve,
      element("span", { className: "warn", textContent: "This site can approve what the agent does for me" })),
      element("p", { textContent: "That does not apply to Canoa on this computer: its agents ask you themselves, and a site cannot answer for them." })]),
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
  const cancel = element("button", { type: "button", className: "quiet", textContent: "Decline" });
  cancel.addEventListener("click", () => send({ acp: "answer", ask: askId, answer: null }));
  // Enter takes the agent's own "allow once", else its first allowing choice, never a standing one.
  const choices = (Array.isArray(options) ? options : []).slice(0, 8);
  const once = choices.findIndex((option) => option.kind === "allow_once");
  const first = choices.findIndex((option) => String(option.kind ?? "").startsWith("allow"));
  answerKeys(buttons[once >= 0 ? once : first] ?? null, cancel);
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
  answerKeys(allow, refuse);
  const who = details.via === "program"
    ? [element("p", {}, element("strong", { textContent: `A program on ${details.label}` }), ` asks to ${WHAT[details.capability] ?? details.capability}.`),
      element("p", { className: "warn", textContent: "Canoa cannot tell which program. Whatever can use that machine's key, which is whatever runs as its user there, can ask here and shares what you allow." }),
      element("p", {}, element("code", { textContent: details.peer }))]
    : [element("p", {}, element("strong", { textContent: details.label }), ` asks to ${WHAT[details.capability] ?? details.capability}.`),
      ...(details.programs ? [element("p", { className: "warn", textContent: "You turned on programs for that machine, so they can use what you allow here too." })] : [])];
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
  const reasons = await send({ acp: "reachable" });
  const agentRows = agents.map((agent) => {
    const remove = element("button", { type: "button", className: "quiet", textContent: "Remove" });
    remove.addEventListener("click", async () => {
      await send({ acp: "remove", key: agent.key });
      manageView();
    });
    const method = agent.peer ? ` (${agent.method ?? "webcrypto"})` : "";
    const why = agent.peer ? reasons?.[agent.method ?? "webcrypto"] : null;
    const unreachable = why ? [element("br"), element("span", { className: "warn", textContent: `Unreachable from this browser: ${why}` })] : [];
    return element("li", {}, element("strong", { textContent: agent.label }), " ", element("code", { textContent: `${agent.url} ${agent.peer ?? "no peer id"}` }), method, ...unreachable, element("br"), remove);
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
  const named = (await readBrowserName(chrome.storage.local).catch(() => null)) ?? defaultBrowserName(navigator.userAgentData);
  main.replaceChildren(element("h1", { textContent: "Canoa agents" }), element("p", {}, "This browser: ", element("strong", { textContent: named })), element("ul", {}, ...agentRows),
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
    pairForm(() => manageView()),
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
