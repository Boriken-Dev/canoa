import 'dart:convert';

/// What a window opened for a conversation is told when it starts: which link to attach by and how to draw.
final class PopoutLaunch {
  const PopoutLaunch({required this.link, required this.title, this.look = 'canoa', this.density = 'normal', this.session});

  /// The key the session is known by in the browser extension's worker, when this window runs there.
  final String? session;

  /// A name both ends of the link know, which only the window that opened this one and this one hold.
  final String link;
  final String title;

  /// The names of the app's own look and density.
  final String look, density;

  /// What marks an argument string as a pop-out window's, among the windows an app opens.
  static const String kind = 'canoa.popout';

  String encode() => jsonEncode({'kind': kind, 'link': link, 'title': title, 'look': look, 'density': density, if (session != null) 'session': session});

  /// The launch [text] holds, or null when it is not one.
  static PopoutLaunch? decode(String text) {
    try {
      final json = jsonDecode(text);
      if (json is! Map || json['kind'] != kind || json['link'] is! String) return null;
      return PopoutLaunch(
        link: json['link'] as String,
        title: json['title'] is String ? json['title'] as String : 'Canoa',
        look: json['look'] is String ? json['look'] as String : 'canoa',
        density: json['density'] is String ? json['density'] as String : 'normal',
        session: json['session'] is String ? json['session'] as String : null,
      );
    } on FormatException {
      return null;
    }
  }
}
