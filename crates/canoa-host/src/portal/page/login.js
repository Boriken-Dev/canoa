// The sign-in page's script, under `script-src 'self'`. It counts a 429's wait down and holds the sign-in buttons until it is over,
// and it runs the passkey ceremonies: a form marked data-passkey asks the portal for options (data-options-url, with the form
// body in data-options-body), has the browser sign or create, and posts the answer with the form. The page works without it:
// everything it reveals is hidden until it runs, and every other way in is an ordinary form.
(function () {
  "use strict";

  function countdown() {
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
  }

  function decode(text) {
    var base = text.replace(/-/g, "+").replace(/_/g, "/");
    while (base.length % 4) base += "=";
    var raw = atob(base);
    var bytes = new Uint8Array(raw.length);
    for (var i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
    return bytes.buffer;
  }

  function encode(buffer) {
    var bytes = new Uint8Array(buffer);
    var raw = "";
    for (var i = 0; i < bytes.length; i++) raw += String.fromCharCode(bytes[i]);
    return btoa(raw).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }

  function extensions(credential) {
    return credential.getClientExtensionResults ? credential.getClientExtensionResults() : {};
  }

  // The answer a browser signs for a sign-in, in the shape WebAuthn's JSON gives it.
  function get(options) {
    var key = options.publicKey;
    key.challenge = decode(key.challenge);
    (key.allowCredentials || []).forEach(function (each) {
      each.id = decode(each.id);
    });
    return navigator.credentials.get({ publicKey: key }).then(function (credential) {
      var response = credential.response;
      return {
        id: credential.id,
        rawId: encode(credential.rawId),
        type: credential.type,
        response: {
          authenticatorData: encode(response.authenticatorData),
          clientDataJSON: encode(response.clientDataJSON),
          signature: encode(response.signature),
          userHandle: response.userHandle ? encode(response.userHandle) : null,
        },
        clientExtensionResults: extensions(credential),
      };
    });
  }

  // The answer a browser makes for a new passkey.
  function create(options) {
    var key = options.publicKey;
    key.challenge = decode(key.challenge);
    key.user.id = decode(key.user.id);
    (key.excludeCredentials || []).forEach(function (each) {
      each.id = decode(each.id);
    });
    return navigator.credentials.create({ publicKey: key }).then(function (credential) {
      var response = credential.response;
      return {
        id: credential.id,
        rawId: encode(credential.rawId),
        type: credential.type,
        response: {
          attestationObject: encode(response.attestationObject),
          clientDataJSON: encode(response.clientDataJSON),
        },
        clientExtensionResults: extensions(credential),
      };
    });
  }

  function words(error) {
    if (error && error.name === "NotAllowedError") return "The passkey was not used.";
    if (error && error.name === "InvalidStateError") return "This device already holds a passkey for you.";
    return (error && error.message) || "The passkey could not be used.";
  }

  function ceremony(form) {
    var create_ = form.getAttribute("data-passkey") === "create";
    var status = form.querySelector("[data-passkey-status]");
    var button = form.querySelector("button[type=submit]");
    function say(text) {
      if (status) status.textContent = text;
    }
    form.addEventListener("submit", function (event) {
      event.preventDefault();
      button.disabled = true;
      say("");
      fetch(form.getAttribute("data-options-url"), {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": "application/x-www-form-urlencoded" },
        body: form.getAttribute("data-options-body") || "",
      })
        .then(function (response) {
          if (response.status === 429) throw new Error("Too many passkey tries. Wait a moment.");
          if (!response.ok) throw new Error("A passkey cannot be used for this now.");
          return response.json();
        })
        .then(function (made) {
          return create_ ? create(made.options) : get(made.options);
        })
        .then(function (answer) {
          form.elements.answer.value = JSON.stringify(answer);
          form.submit();
        })
        .catch(function (error) {
          button.disabled = false;
          say(words(error));
        });
    });
  }

  function passkeys() {
    var gated = document.querySelectorAll("[data-gated]");
    var forms = document.querySelectorAll("form[data-passkey]");
    var missing = document.querySelectorAll("[data-passkey-missing]");
    var works = !!(window.PublicKeyCredential && navigator.credentials && window.fetch);
    for (var i = 0; i < gated.length; i++) gated[i].hidden = false;
    for (var j = 0; j < forms.length; j++) {
      if (works) ceremony(forms[j]);
      else forms[j].hidden = true;
    }
    for (var k = 0; k < missing.length; k++) missing[k].hidden = works;
  }

  countdown();
  passkeys();
})();
