import 'dart:convert';

import '../brand.dart';
import 'popout_window.dart' show PopoutItem;

/// What a window opened for a conversation is told when it starts: which link to attach by and how to draw.
final class PopoutLaunch {
  const PopoutLaunch({required this.link, required this.title, this.look = 'canoa', this.density = 'normal', this.session, this.item});

  /// The key the session is known by in the browser extension's worker, when this window runs there.
  final String? session;

  /// A name both ends of the link know, which only the window that opened this one and this one hold.
  final String link;
  final String title;

  /// The names of the app's own look and density.
  final String look, density;

  /// What the window draws in place of a conversation; null for a conversation.
  final PopoutItem? item;

  /// What marks an argument string as a pop-out window's, among the windows an app opens.
  static const String kind = 'canoa.popout';

  String encode() => jsonEncode({
    'kind': kind,
    'link': link,
    'title': title,
    'look': look,
    'density': density,
    if (session != null) 'session': session,
    if (item != null) 'item': {'kind': item!.kind, 'args': item!.args},
  });

  /// The launch [text] holds, or null when it is not one.
  static PopoutLaunch? decode(String text) {
    try {
      final json = jsonDecode(text);
      if (json is! Map || json['kind'] != kind || json['link'] is! String) return null;
      final item = json['item'];
      return PopoutLaunch(
        link: json['link'] as String,
        title: json['title'] is String ? json['title'] as String : Brand.name,
        look: json['look'] is String ? json['look'] as String : 'canoa',
        density: json['density'] is String ? json['density'] as String : 'normal',
        session: json['session'] is String ? json['session'] as String : null,
        item: item is Map && item['kind'] is String ? PopoutItem(item['kind'] as String, itemArgs(item['args'])) : null,
      );
    } on FormatException {
      return null;
    }
  }

  /// The text-valued entries of [raw], which is what an item's arguments are.
  static Map<String, String> itemArgs(Object? raw) => {
    if (raw is Map)
      for (final MapEntry(:key, :value) in raw.entries)
        if (key is String && value is String) key: value,
  };
}
