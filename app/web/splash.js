// Takes the loading screen away when Flutter has painted its first frame, and drops a service worker an earlier build
// registered (this one has none), which Flutter's loader would otherwise register again and wait up to 4 seconds for.
(function () {
  if (navigator.serviceWorker) {
    navigator.serviceWorker.getRegistrations().then(function (all) {
      all.forEach(function (registration) {
        registration.unregister();
      });
    }, function () {});
  }
  var splash = document.getElementById("splash");
  if (!splash) return;
  function remove() {
    splash.classList.add("done");
    setTimeout(function () {
      splash.remove();
    }, 300);
  }
  // Captured on the window, since the engine dispatches the event on an element that need not bubble it.
  window.addEventListener("flutter-first-frame", remove, { capture: true, once: true });
})();
