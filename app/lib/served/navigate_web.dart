import 'dart:js_interop';

import 'package:web/web.dart' as web;

import '../client/pane_message.dart';

void navigateTo(Uri address) {
  final parent = web.window.parent;
  if (parent != null && !identical(parent, web.window)) {
    // A frame cannot show the sign-in page (it is never framed); the page that holds this one shows its own.
    parent.postMessage(signInMessage.toJS, web.window.location.origin.toJS);
    return;
  }
  web.window.location.assign(address.toString());
}

void reloadPage() => web.window.location.reload();
