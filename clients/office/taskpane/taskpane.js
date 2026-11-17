// The task pane: reads what is open in Outlook, Word, Excel or PowerPoint through Office.js, works out its relation with
// relation.js, and shows the chat-only app for it in a frame of this same origin. The app is a Flutter page with its own
// router, which Office.js would break (it removes history.pushState and replaceState), so Office.js lives only in this page.
import { appAddress, relationForDocument, relationForMail, sameRelation } from './relation.js';

const frame = document.getElementById('canoa');
const status = document.getElementById('status');
const signIn = document.getElementById('signin');

/** The relation the frame shows now (null for none, undefined before the first), and the address it was given. */
let shown;
let loaded = '';

/** Sets what the bar says. */
function say(text) {
  status.textContent = text;
}

/** Shows the chat for `relation`; the same relation again changes nothing, so a pinned pane does not reload on a reselect. */
function show(relation) {
  if (shown !== undefined && sameRelation(shown, relation)) return;
  shown = relation;
  const address = appAddress(window.location.origin, relation);
  if (address !== loaded) {
    loaded = address;
    frame.src = address;
  }
  say(relation ? `Sessions of ${relation.label ?? 'this item'} first` : 'Not tied to an item');
}

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

/** Opens the sign-in page in an Office dialog; the string it sends back ends it and starts the chat again. */
function openSignIn() {
  const page = new URL('/office/dialog.html', window.location.origin);
  page.searchParams.set('start', '/');
  Office.context.ui.displayDialogAsync(page.href, { height: 60, width: 30 }, (opened) => {
    if (opened.status !== Office.AsyncResultStatus.Succeeded) {
      say(`The sign-in window did not open (${opened.error.code})`);
      return;
    }
    const dialog = opened.value;
    dialog.addEventHandler(Office.EventType.DialogMessageReceived, () => {
      // A string and nothing else comes back, and what it holds is the sign-in page's to keep: it is neither run nor stored here.
      dialog.close();
      loaded = '';
      shown = undefined;
      void refresh();
    });
  });
}

// Office.js defines `Office` before this module runs; without it (a browser tab, a blocked script) the pane says so.
if (typeof Office === 'undefined') {
  say('Office.js did not load');
} else {
  Office.onReady(async (info) => {
    signIn.hidden = false;
    signIn.addEventListener('click', openSignIn);
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
    await refresh();
  });
}
