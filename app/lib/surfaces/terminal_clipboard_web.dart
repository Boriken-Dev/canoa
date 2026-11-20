import 'dart:js_interop';

import 'package:flutter/foundation.dart' show VoidCallback;
import 'package:web/web.dart' as web;

import 'terminal_clipboard.dart' show ClipboardRefused;

/// A browser delivers a keyboard paste as a `paste` event on the page, which needs no permission; reading the clipboard from a
/// script does, and is refused outside a secure context.
const pastesByEvent = true;

const _usePasteKey = 'Paste with the keyboard instead (Ctrl+V, or Cmd+V on a Mac), which needs no permission';

/// Writes with the clipboard API where the page may use it, else with a copy command on a hidden field; both need the person's own
/// key or click to be running.
Future<void> writeText(String text) async {
  if (web.window.isSecureContext) {
    try {
      await web.window.navigator.clipboard.writeText(text).toDart;
      return;
    } on Object {
      // Refused (no focus, no gesture): the copy command below still has the gesture the caller is running in.
    }
  }
  if (!_copyCommand(text)) {
    throw const ClipboardRefused('The browser did not let this page copy the text. Try again from the keyboard or the menu.');
  }
}

bool _copyCommand(String text) {
  final body = web.document.body;
  if (body == null) return false;
  final holder = web.document.createElement('textarea') as web.HTMLTextAreaElement;
  holder.value = text;
  holder.setAttribute('readonly', '');
  holder.style
    ..position = 'fixed'
    ..left = '-9999px'
    ..opacity = '0';
  final before = web.document.activeElement;
  body.append(holder);
  try {
    holder.select();
    return web.document.execCommand('copy');
  } finally {
    holder.remove();
    if (before case final web.HTMLElement element) element.focus();
  }
}

Future<String?> readText() async {
  if (!web.window.isSecureContext) {
    throw const ClipboardRefused('The browser only lets a page read the clipboard over HTTPS or on localhost. $_usePasteKey.');
  }
  try {
    final text = (await web.window.navigator.clipboard.readText().toDart).toDart;
    return text.isEmpty ? null : text;
  } on Object {
    throw const ClipboardRefused('The browser did not let this page read the clipboard. $_usePasteKey, or allow this site to read the clipboard.');
  }
}

/// Listens for `paste` on the whole page, before the page's own handlers; a paste the handler takes is not given to them, so the
/// browser's hidden text field does not type it a second time.
VoidCallback listenPaste(bool Function(String text) handler) {
  final listener = ((web.Event event) {
    final text = (event as web.ClipboardEvent).clipboardData?.getData('text/plain') ?? '';
    if (text.isNotEmpty && handler(text)) {
      event.preventDefault();
      event.stopImmediatePropagation();
    }
  }).toJS;
  web.document.addEventListener('paste', listener, true.toJS);
  return () => web.document.removeEventListener('paste', listener, true.toJS);
}
