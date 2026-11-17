// The task pane: reads what is open in Outlook, Word, Excel or PowerPoint through Office.js, works out its relation with
// relation.js, and shows the chat-only app for it in a frame of this same origin. The app is a Flutter page with its own
// router, which Office.js would break (it removes history.pushState and replaceState), so Office.js lives only in this page.
//
// Behind a portal the frame needs a session of its own: the site's cookie is not sent from inside Office's page, and the dialog
// that signs in shares no cookies with this frame. The dialog sends a string, this page hands it to the host, and the answer sets
// the frame's cookie. A host with no sign-in answers `GET /office/session` with its own page, and the chat is shown as it is.
import {
  appAddress,
  handoffFrom,
  parsePaneMessage,
  portalState,
  relationForDocument,
  relationForMail,
  relationMessage,
  sameRelation,
} from './relation.js';

const frame = document.getElementById('canoa');
const status = document.getElementById('status');
const signIn = document.getElementById('signin');
const origin = window.location.origin;

/** The relation the frame shows now (null for none, undefined before the first). */
let shown;
/** Whether the app in the frame has said it listens for a relation, so a change goes to it and not into a reload. */
let listening = false;
/** Whether the frame has been given the app's address yet. */
let loaded = false;
/** `open` (no sign-in on this host), `signed-in` or `signed-out`, from the host's answer. */
let gate = 'open';

/** Sets what the bar says. */
function say(text) {
  status.textContent = text;
}

/** What the bar says of a relation. */
function describe(relation) {
  return relation ? `Sessions of ${relation.label ?? 'this item'} first` : 'Not tied to an item';
}

/**
 * Shows the chat for `relation`. The first time the frame is loaded with the relation in its address; after that the app is told
 * through a message and keeps its place, so a pinned pane moving between messages does not reload the chat. The same relation
 * again changes nothing.
 */
function show(relation) {
  if (gate === 'signed-out') return;
  if (shown !== undefined && sameRelation(shown, relation)) return;
  shown = relation;
  say(describe(relation));
  if (!loaded) {
    loaded = true;
    listening = false;
    frame.hidden = false;
    frame.src = appAddress(origin, relation);
    return;
  }
  tell(relation);
}

/** Tells the app what is in front, when it is listening; it asks for it by `ready` when it starts. */
function tell(relation) {
  if (listening && frame.contentWindow) frame.contentWindow.postMessage(relationMessage(relation), origin);
}

/** Shows the pane as not signed in: no chat, and the button that opens the dialog. */
function needSignIn() {
  gate = 'signed-out';
  shown = undefined;
  listening = false;
  loaded = false;
  frame.hidden = true;
  signIn.hidden = false;
  say('Sign in to use Canoa here');
}

/** Whether this host keeps a sign-in and the frame has one. A host that answers nothing the portal would is taken as open. */
async function checkGate() {
  try {
    const answer = await fetch('/office/session', { credentials: 'same-origin', cache: 'no-store' });
    gate = portalState(answer.status, answer.headers.get('content-type'));
  } catch {
    gate = 'open';
  }
  signIn.hidden = gate !== 'signed-out';
  if (gate === 'signed-out') needSignIn();
}

// The app says `ready` when it listens (and so again after it reloads), and `signin` when its session has ended.
window.addEventListener('message', (event) => {
  if (event.origin !== origin || event.source !== frame.contentWindow) return;
  const said = parsePaneMessage(event.data);
  if (said === 'ready') {
    listening = true;
    if (shown !== undefined) tell(shown);
  } else if (said === 'signin') {
    needSignIn();
  }
});

/** The open mail item's subject: a string when reading, a getter when composing. */
function subjectOf(item) {
  const subject = item.subject;
  if (typeof subject === 'string') return Promise.resolve(subject);
  if (subject && typeof subject.getAsync === 'function') {
    return new Promise((resolve) => {
      subject.getAsync((result) => resolve(result.status === Office.AsyncResultStatus.Succeeded ? result.value : undefined));
    });
  }
  return Promise.resolve(undefined);
}

/** The relation of what Outlook has open now. The item is null when the pane is pinned and nothing is selected. */
async function mailRelation() {
  const item = Office.context.mailbox.item;
  if (!item) return null;
  return relationForMail({ conversationId: item.conversationId, subject: await subjectOf(item) });
}

/** The relation of the open document, for the Office application `app`. */
function documentRelation(app) {
  return relationForDocument({ app, url: Office.context.document.url });
}

let refresh = async () => {};

/** How often a pane on the host's own computer asks whether the host still answers. */
const WATCH_MS = 5000;

/** What this host says about itself at `/canoa.json`, or null when it does not answer. */
async function hostIndex() {
  try {
    const response = await fetch('/canoa.json', { cache: 'no-store' });
    return response.ok ? await response.json() : null;
  } catch {
    return null;
  }
}

/** Whether the host offers to accept this page's key itself, which it does only to a page on its own computer. */
let onThisComputer = false;
const indexAtLoad = hostIndex().then((index) => {
  onThisComputer = typeof index?.admit === 'string';
  return index;
});

/**
 * On the host's own computer there is nothing to sign in to and nothing to pair: the pane says when that Canoa stops answering
 * and loads the chat again when it does. A page cannot start a program, so it says what starts one.
 */
function watchHost() {
  let down = false;
  setInterval(async () => {
    const answered = (await hostIndex()) !== null;
    if (!answered && !down) {
      down = true;
      say('Canoa is not running on this computer. Start the Canoa app, or run "canoa serve --web": this pane reconnects.');
    } else if (answered && down) {
      down = false;
      loaded = false;
      shown = undefined;
      listening = false;
      await checkGate();
      if (gate !== 'signed-out') await refresh();
    }
  }, WATCH_MS);
}

/** Hands the dialog's string to the host, which opens this frame's session. Whether it did. */
async function claim(string) {
  try {
    const answer = await fetch('/office/session', {
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ handoff: string }),
    });
    return answer.ok;
  } catch {
    return false;
  }
}

/** Opens the sign-in in an Office dialog; the string it sends back is claimed and the chat starts. */
function openSignIn() {
  const page = new URL('/office/dialog.html', origin);
  page.searchParams.set('start', '/office/signin');
  Office.context.ui.displayDialogAsync(page.href, { height: 60, width: 30 }, (opened) => {
    if (opened.status !== Office.AsyncResultStatus.Succeeded) {
      say(`The sign-in window did not open (${opened.error.code})`);
      return;
    }
    const dialog = opened.value;
    dialog.addEventHandler(Office.EventType.DialogMessageReceived, async (arrived) => {
      // A string and nothing else comes back. It is only passed on when it has the shape the host makes, and never run or stored.
      dialog.close();
      const string = handoffFrom(arrived.message);
      if (string === null) {
        say('The sign-in did not finish: try again');
        return;
      }
      say('Signing in');
      if (!(await claim(string))) {
        say('The sign-in did not take: try again');
        return;
      }
      gate = 'signed-in';
      signIn.hidden = true;
      shown = undefined;
      loaded = false;
      await refresh();
    });
  });
}

// Office.js defines `Office` before this module runs; without it (a browser tab, a blocked script) the pane says so.
if (typeof Office === 'undefined') {
  say('Office.js did not load');
} else {
  Office.onReady(async (info) => {
    await indexAtLoad;
    signIn.addEventListener('click', openSignIn);
    if (onThisComputer) watchHost();
    if (info.host === Office.HostType.Outlook) {
      refresh = async () => show(await mailRelation());
      // A pinned pane stays open while the person moves between messages; the item is already the new one when this runs.
      Office.context.mailbox.addHandlerAsync(Office.EventType.ItemChanged, () => void refresh());
    } else if (info.host !== undefined && info.host !== null) {
      const app = String(info.host).toLowerCase();
      refresh = async () => show(await documentRelation(app));
    } else {
      say('This Office application is not known');
      return;
    }
    await checkGate();
    if (gate !== 'signed-out') await refresh();
  });
}
