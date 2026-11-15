// Counts a 429's wait down and holds the sign-in buttons until it is over. The page works without it.
(function () {
  var main = document.querySelector("main[data-retry]");
  if (!main) return;
  var left = parseInt(main.getAttribute("data-retry"), 10);
  if (!(left > 0)) return;
  var buttons = document.querySelectorAll("button[type=submit]");
  var note = document.getElementById("wait");
  function show() {
    var wait = left > 0;
    for (var i = 0; i < buttons.length; i++) buttons[i].disabled = wait;
    if (note) {
      note.textContent = wait
        ? "Try again in " + Math.floor(left / 60) + ":" + ("0" + (left % 60)).slice(-2) + "."
        : "You can try again.";
    }
  }
  show();
  var timer = setInterval(function () {
    left -= 1;
    show();
    if (left <= 0) clearInterval(timer);
  }, 1000);
})();
