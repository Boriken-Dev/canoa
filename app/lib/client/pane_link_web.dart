import 'dart:async';
import 'dart:js_interop';

import 'package:web/web.dart' as web;

import 'pane_link.dart';
import 'pane_message.dart';
import 'relation.dart';

/// The link to the page that frames this one, when one does: only a page of this origin is heard or spoken to.
PaneLink? paneLink() {
  final parent = web.window.parent;
  return parent == null || identical(parent, web.window) ? null : _WebPaneLink(parent);
}

final class _WebPaneLink implements PaneLink {
  _WebPaneLink(this._parent) {
    web.window.addEventListener('message', _heard);
  }

  final web.Window _parent;
  final _relations = StreamController<Relation?>.broadcast();

  late final JSFunction _heard = ((web.MessageEvent event) {
    // The origin is this page's own and the sender is the page that frames it: nothing else may change what is shown.
    if (event.origin != web.window.location.origin || !identical(event.source, _parent)) return;
    final data = event.data;
    final message = parseRelationMessage(data.isA<JSString>() ? (data as JSString).toDart : null);
    if (message != null) _relations.add(message.relation);
  }).toJS;

  void _send(String message) => _parent.postMessage(message.toJS, web.window.location.origin.toJS);

  @override
  Stream<Relation?> get relations => _relations.stream;

  @override
  void ready() => _send(readyMessage);

  @override
  void signIn() => _send(signInMessage);

  @override
  void dispose() {
    web.window.removeEventListener('message', _heard);
    unawaited(_relations.close());
  }
}
