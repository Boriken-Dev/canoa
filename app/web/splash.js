// Takes the loading screen away when Flutter has painted its first frame.
(function () {
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
