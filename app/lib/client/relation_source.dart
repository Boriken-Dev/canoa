import 'package:flutter/foundation.dart';

import '../extension/tab_shares.dart';
import 'relation.dart';

/// What is in front of the client now, which the sessions related to it are listed for. The extension's is the page in the tab in
/// front; an Office task pane gives the open mail conversation or document through a [ManualRelation].
abstract interface class RelationSource implements Listenable {
  /// What is in front, or null when it is nothing a session can relate to.
  Relation? get current;
}

/// The page in the tab in front, by its origin.
final class FrontTabRelation extends ChangeNotifier implements RelationSource {
  FrontTabRelation(this.tabs) {
    tabs.addListener(notifyListeners);
  }

  final TabShares tabs;

  @override
  Relation? get current {
    final front = tabs.frontTab;
    return front == null ? null : originRelation(front.url, title: front.title);
  }

  @override
  void dispose() {
    tabs.removeListener(notifyListeners);
    super.dispose();
  }
}

/// A relation the client sets, and sets again when what is in front changes.
final class ManualRelation extends ChangeNotifier implements RelationSource {
  ManualRelation([this._current]);

  Relation? _current;

  @override
  Relation? get current => _current;

  set current(Relation? next) {
    if (next == _current) return;
    _current = next;
    notifyListeners();
  }
}
