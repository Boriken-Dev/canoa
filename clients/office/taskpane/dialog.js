// The Office dialog's page. Two ways in, both on this origin:
//   /office/dialog.html?start=/path      goes on to that page of this site, which is the sign-in;
//   /office/dialog.html#done=<string>    is where a sign-in page ends: the string goes to the task pane and the dialog closes.
// A dialog is a separate browser instance, so a string through messageParent is the only thing that can reach the pane.
(function () {
  var status = document.getElementById('status');
  var MAX = 2048;

  function say(text) {
    status.textContent = text;
  }

  /** The string after `done=` in the fragment, or null. */
  function doneString() {
    var match = /^#done=(.*)$/.exec(window.location.hash);
    if (!match) return null;
    try {
      return decodeURIComponent(match[1]);
    } catch (error) {
      return null;
    }
  }

  /** A path on this site and nothing else: one slash, then not another slash or a backslash. */
  function samePath(path) {
    return typeof path === 'string' && /^\/(?![/\\])/.test(path);
  }

  if (typeof Office === 'undefined') {
    say('Office.js did not load');
    return;
  }
  Office.onReady(function () {
    var done = doneString();
    if (done !== null) {
      if (done.length > MAX) {
        say('The sign-in result is too long');
        return;
      }
      Office.context.ui.messageParent(done);
      return;
    }
    var start = new URLSearchParams(window.location.search).get('start');
    if (samePath(start)) {
      window.location.assign(start);
    } else {
      say('No sign-in page was named');
    }
  });
})();
