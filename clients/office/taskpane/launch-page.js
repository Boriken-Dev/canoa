// The page the launcher shows in place of the task pane while no Canoa runs: it says so, offers to start it, and loads the pane
// again once a host answers. Nothing else of the add-in is here.
import { keyOf, offerStart, waitForHost } from './launch.js';

const status = document.getElementById('status');
const start = document.getElementById('start');
const key = keyOf(window.location.search);

start.addEventListener('click', async () => {
  start.disabled = true;
  status.textContent = 'Starting Canoa';
  const result = await offerStart({ key });
  status.textContent = result.said;
  if (result.ok) {
    window.location.reload();
  } else {
    start.disabled = false;
  }
});

// A Canoa the person starts some other way is noticed too.
(async () => {
  if (await waitForHost({ tries: 3600, every: 2000 })) window.location.reload();
})();
