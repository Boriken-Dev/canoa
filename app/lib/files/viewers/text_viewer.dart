import 'package:flutter/material.dart';

import '../../theme/look.dart';
import 'file_viewer.dart';
import 'file_sample.dart';

/// [text] with every control, format, bidirectional or invisible character but a line break or tab shown as `<U+XXXX>`, so what a
/// person reads is what the file says; and a run of combining marks cut after [markRun], the rest shown the same way. The host leaves
/// such names out of its listings; this is for what it cannot know, the text.
String visibleText(String text) {
  final out = StringBuffer();
  var run = 0;
  for (final rune in text.runes) {
    final combining = _combines(rune);
    run = combining ? run + 1 : 0;
    if (_hides(rune) || (combining && run > markRun)) {
      out.write('<U+${rune.toRadixString(16).toUpperCase().padLeft(4, '0')}>');
    } else {
      out.writeCharCode(rune);
    }
  }
  return out.toString();
}

/// How many combining marks may follow one character before the rest are shown as code points: enough for any language, too few to
/// stack a line of text out of its row.
const int markRun = 3;

/// Whether [c] is a combining mark.
bool _combines(int c) =>
    (c >= 0x0300 && c <= 0x036f) ||
    (c >= 0x0483 && c <= 0x0489) ||
    (c >= 0x0591 && c <= 0x05bd) ||
    (c >= 0x064b && c <= 0x065f) ||
    (c >= 0x1ab0 && c <= 0x1aff) ||
    (c >= 0x1dc0 && c <= 0x1dff) ||
    (c >= 0x20d0 && c <= 0x20ff) ||
    (c >= 0xfe20 && c <= 0xfe2f);

/// The characters the host's own list of hidden text names (`store::text`), and the control characters but a line break, a carriage
/// return and a tab.
bool _hides(int c) =>
    c <= 0x08 ||
    c == 0x0b ||
    c == 0x0c ||
    (c >= 0x0e && c <= 0x1f) ||
    (c >= 0x7f && c <= 0x9f) ||
    c == 0xad ||
    c == 0x034f ||
    c == 0x061c ||
    c == 0x115f ||
    c == 0x1160 ||
    (c >= 0x17b4 && c <= 0x17b5) ||
    c == 0x180e ||
    (c >= 0x200b && c <= 0x200f) ||
    c == 0x2028 ||
    c == 0x2029 ||
    (c >= 0x202a && c <= 0x202e) ||
    (c >= 0x2060 && c <= 0x206f) ||
    c == 0x3164 ||
    (c >= 0xfe00 && c <= 0xfe0f) ||
    c == 0xfeff ||
    c == 0xffa0 ||
    (c >= 0xfff9 && c <= 0xfffb) ||
    (c >= 0x1d173 && c <= 0x1d17a) ||
    (c >= 0xe0000 && c <= 0xe007f) ||
    (c >= 0xe0100 && c <= 0xe01ef);

/// The lines of [text] as shown: each cut into pieces of at most [lineLength] characters, so no one line is a layout of the whole
/// file, with a carriage return before a line break dropped.
List<String> previewLines(String text) {
  final lines = <String>[];
  for (final line in text.split('\n')) {
    final bare = line.endsWith('\r') ? line.substring(0, line.length - 1) : line;
    if (bare.length <= lineLength) {
      lines.add(bare);
      continue;
    }
    var at = 0;
    while (at < bare.length) {
      var end = at + lineLength > bare.length ? bare.length : at + lineLength;
      // Not between the halves of one character.
      if (end < bare.length && bare.codeUnitAt(end) >= 0xdc00 && bare.codeUnitAt(end) <= 0xdfff) {
        end--;
      }
      lines.add(bare.substring(at, end));
      at = end;
    }
  }
  return lines;
}

/// The longest piece of a line the preview lays out at once.
const int lineLength = 400;

/// [text] as read-only monospace lines, laid out as they scroll into view.
class TextLines extends StatelessWidget {
  const TextLines({super.key, required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    final lines = previewLines(text);
    return ListView.builder(
      key: const ValueKey('files text'),
      padding: const EdgeInsets.all(16),
      itemCount: lines.length,
      itemBuilder: (context, at) => SelectableText(
        visibleText(lines[at]),
        key: ValueKey('files line $at'),
        style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12),
      ),
    );
  }
}

/// Any text, as it is written: the viewer a file with no better one is shown by.
class TextViewer extends FileViewer {
  const TextViewer();

  static const String viewerId = 'text';

  @override
  String get id => viewerId;
  @override
  String get labelKey => 'files.viewer.text';
  @override
  int get priority => -10;

  @override
  bool canShow(FileSample sample) => sample.isText;

  @override
  bool matches(FileSample sample, String type) => canShow(sample);

  @override
  Widget build(BuildContext context, FileViewRequest request) => TextLines(text: request.sample.text ?? '');
}
