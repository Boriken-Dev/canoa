// The paired agents a person can pick on the permissions page, so no one pastes a key. No chrome.* here, so it can be tested alone.
const PEER = /^sha256:[0-9a-f]{64}$/;

const clean = (value) => String(value ?? "").replace(/\p{Cf}|\p{Cc}/gu, " ").trim().slice(0, 80);

/** One choice per paired agent: its peer id is the value, its name and the start of the id what the person reads. */
export function agentChoices(machines) {
  if (!Array.isArray(machines)) return [];
  return machines
    .filter((machine) => PEER.test(machine?.peer))
    .map((machine) => ({ value: machine.peer, label: `${clean(machine.label) || "Unnamed"} — ${machine.peer.slice(0, 20)}…` }));
}

/** The worker's check: a grant names an agent this browser holds a key for, whatever a page sent. */
export function isPairedPeer(peers, peerId) {
  return Array.isArray(peers) && PEER.test(peerId) && peers.includes(peerId);
}
