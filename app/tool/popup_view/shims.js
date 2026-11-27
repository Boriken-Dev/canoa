// The framework-side shims for a Flutter view in a window.open popup, each switched on by a letter of `?shims=` on the page's address:
//   a  keydown and keyup of the popup become opener-realm KeyboardEvents dispatched on the opener's window; defaultPrevented is copied back
//   b  the popup's selectionchange is forwarded to the opener's document
//   c  the opener's KeyboardEvent, CompositionEvent, InputEvent, PointerEvent, HTMLInputElement and HTMLTextAreaElement accept the
//      popups' instances in instanceof
//   d  the opener's document.hasFocus is true while any popup of the app has focus
//   e  the opener's requestAnimationFrame also asks a visible popup, so frames run while the opener is hidden
//   g  the opener's document.activeElement is the active element of the app window that took the focus last (the engine compares it
//      with its views when the framework moves focus)
//   h  (with a) the browser's own Tab is always prevented in the popup: the framework moves the focus itself
//   f  the opener's document.visibilityState and hidden read visible while a popup is visible (the engine's lifecycle state turns
//      hidden with them, and the framework then stops producing frames)
// `probeShimsPatch(flags)` runs once in the opener before the engine starts; `probeShims.install(popup)` runs per popup.
(() => {
  const popups = [];
  let flags = '';
  const on = (letter) => flags.includes(letter);
  const live = () => popups.filter((popup) => !popup.closed);
  const realState = () => Object.getOwnPropertyDescriptor(Document.prototype, 'visibilityState').get.call(document);

  const widen = (name) => {
    const own = window[name];
    const sameKind = (instance) => live().some((popup) => {
      try {
        return popup[name] && Function.prototype[Symbol.hasInstance].call(popup[name], instance);
      } catch {
        return false;
      }
    });
    Object.defineProperty(own, Symbol.hasInstance, {
      configurable: true,
      value: (instance) => Function.prototype[Symbol.hasInstance].call(own, instance) || sameKind(instance),
    });
  };

  // The events the text editing depends on, as the documents see them, for a failed check's report.
  const trail = [];
  const watch = (where, target) => {
    target.addEventListener('keydown', (event) => trail.push(`${where}:keydown-after-engine:${event.key}:prevented=${event.defaultPrevented}`), false);
    for (const type of ['compositionstart', 'compositionupdate', 'compositionend', 'beforeinput', 'input', 'focusin', 'focusout', 'selectionchange']) {
      target.addEventListener(type, (event) => {
        trail.push(`${where}:${type}:${event.target?.tagName ?? '-'}${event.data ? `:${event.data}` : ''}`);
        if (trail.length > 400) trail.shift();
      }, true);
    }
  };
  window.probeTrail = (count) => trail.slice(-count).join(' ');
  window.realVisibility = realState;

  window.probeShimsPatch = (given) => {
    flags = given;
    watch('opener', document);
    if (on('c')) for (const name of ['KeyboardEvent', 'CompositionEvent', 'InputEvent', 'PointerEvent', 'HTMLInputElement', 'HTMLTextAreaElement']) widen(name);
    if (on('d')) {
      const own = document.hasFocus.bind(document);
      document.hasFocus = () => own() || live().some((popup) => popup.document.hasFocus());
    }
    if (on('g')) {
      const own = Object.getOwnPropertyDescriptor(Document.prototype, 'activeElement').get;
      let last = null;
      const note = (target) => target.addEventListener('focusin', () => { last = target; }, true);
      note(document);
      window.probeNoteFocus = note;
      Object.defineProperty(document, 'activeElement', {
        configurable: true,
        get() {
          if (last && last !== document && !(last.defaultView?.closed ?? true)) return last.activeElement;
          return own.call(document);
        },
      });
    }
    if (on('f')) {
      const shown = () => live().some((popup) => popup.document.visibilityState === 'visible');
      Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => (shown() ? 'visible' : realState()) });
      Object.defineProperty(document, 'hidden', { configurable: true, get: () => (shown() ? false : realState() === 'hidden') });
    }
    if (on('e')) {
      const own = window.requestAnimationFrame.bind(window);
      // A request made while the opener was shown stays pending when it is hidden, and the engine schedules no other frame until
      // it runs: the pending ones are asked of a visible popup too when the opener hides. The first of the two to run wins.
      const pending = new Set();
      const shownPopup = () => live().find((popup) => popup.document.visibilityState === 'visible');
      window.requestAnimationFrame = (callback) => {
        let done = false;
        const run = () => {
          if (done) return;
          done = true;
          pending.delete(run);
          callback(performance.now());
        };
        pending.add(run);
        const visible = shownPopup();
        if (realState() === 'hidden' && visible) visible.requestAnimationFrame(run);
        return own(run);
      };
      document.addEventListener('visibilitychange', () => {
        const visible = shownPopup();
        if (realState() === 'hidden' && visible) for (const run of pending) visible.requestAnimationFrame(run);
      });
    }
  };

  window.probeShims = {
    install(popup) {
      popups.push(popup);
      if (on('g')) window.probeNoteFocus(popup.document);
      watch('popup', popup.document);
      if (on('a')) {
        for (const type of ['keydown', 'keyup']) {
          popup.addEventListener(type, (event) => {
            const copy = new KeyboardEvent(type, {
              key: event.key, code: event.code, location: event.location, repeat: event.repeat, isComposing: event.isComposing,
              ctrlKey: event.ctrlKey, shiftKey: event.shiftKey, altKey: event.altKey, metaKey: event.metaKey,
              keyCode: event.keyCode, charCode: event.charCode, bubbles: true, cancelable: true, composed: true,
            });
            window.dispatchEvent(copy);
            if (copy.defaultPrevented || (on('h') && event.key === 'Tab')) event.preventDefault();
            trail.push(`key:${type}:${event.key}:prevented=${copy.defaultPrevented}`);
          }, true);
        }
      }
      if (on('b')) popup.document.addEventListener('selectionchange', () => document.dispatchEvent(new Event('selectionchange')));
    },
  };
})();
