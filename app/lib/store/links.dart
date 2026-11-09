import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:url_launcher/url_launcher.dart';

/// Opens [uri] in the system's handler for it; true when one took it. Tests replace it.
Future<bool> Function(Uri uri) openLink = (uri) => launchUrl(uri, mode: LaunchMode.externalApplication);

/// [text] as an address a person may open: a plain `http` or `https` URL with a host, no user information, and printable
/// ASCII only, so a look-alike host or a hidden character cannot pass. Anything else stays text, since a catalog's or a
/// manifest's words are another's.
Uri? webLink(String text) {
  if (!RegExp(r'^[!-~]+$').hasMatch(text)) return null;
  final uri = Uri.tryParse(text);
  if (uri == null || !uri.hasScheme || uri.host.isEmpty || uri.userInfo.isNotEmpty) return null;
  return uri.scheme == 'http' || uri.scheme == 'https' ? uri : null;
}

/// A labelled address of another's that opens when tapped: shown whole, so what opens is what is read, and never fetched
/// by Canoa.
class LinkField extends StatefulWidget {
  const LinkField({super.key, required this.label, required this.uri, required this.shown});

  final String label;
  final Uri uri;

  /// The address as it reads on screen.
  final String shown;

  @override
  State<LinkField> createState() => _LinkFieldState();
}

class _LinkFieldState extends State<LinkField> {
  late final TapGestureRecognizer _tap = TapGestureRecognizer()..onTap = () => openLink(widget.uri);

  @override
  void dispose() {
    _tap.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 4),
    child: Text.rich(TextSpan(children: [
      TextSpan(text: '${widget.label}: ', style: const TextStyle(fontWeight: FontWeight.w600)),
      TextSpan(
        text: widget.shown,
        recognizer: _tap,
        mouseCursor: SystemMouseCursors.click,
        semanticsLabel: '${widget.label} ${widget.shown}',
        style: TextStyle(color: Theme.of(context).colorScheme.primary, decoration: TextDecoration.underline),
      ),
    ])),
  );
}
