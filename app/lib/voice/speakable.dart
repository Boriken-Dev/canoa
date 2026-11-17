import 'package:markdown/markdown.dart' as md;

/// What a reply says in words, for reading aloud: its prose, with the formatting taken off and nothing of the code blocks, which are
/// not read, nor of images, which say nothing a voice can. Code that sits in a sentence is read as its text and a link as its label.
/// Blocks end in a full stop, so a heading and the paragraph after it are not run together.
String spokenText(String markdown) => _prose(markdown).replaceAll(RegExp(r'\n{2,}'), '\n').trim();

/// [spokenText] with a blank line where a block that is not read stood, so the sentence before it is known to be over while the
/// block is still being written.
String _prose(String markdown) {
  final nodes = md.Document(extensionSet: md.ExtensionSet.gitHubFlavored).parseLines(markdown.split('\n'));
  final blocks = <String>[];
  for (final node in nodes) {
    _block(node, blocks);
  }
  return blocks.join('\n');
}

const _blockTags = {'p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'li', 'blockquote', 'ul', 'ol', 'table', 'tr', 'thead', 'tbody', 'details'};

void _block(md.Node node, List<String> out) {
  if (node is md.Text) {
    final text = _squash(node.text);
    if (text.isNotEmpty) out.add(text);
    return;
  }
  if (node is! md.Element) return;
  switch (node.tag) {
    case 'pre' || 'hr' || 'img':
      if (node.tag == 'pre') out.add('');
      return;
    case 'table':
      // Cells are read row by row, each row as a sentence.
      void rows(md.Node n) {
        if (n is! md.Element) return;
        if (n.tag == 'tr') {
          final cells = [
            for (final cell in n.children ?? const <md.Node>[])
              if (cell is md.Element && _inline(cell).isNotEmpty) _inline(cell),
          ];
          if (cells.isNotEmpty) out.add('${cells.join(', ')}.');
        } else {
          n.children?.forEach(rows);
        }
      }

      rows(node);
      return;
    case 'ul' || 'ol' || 'blockquote' || 'details' || 'li' when _hasBlockChild(node):
      for (final child in node.children ?? const <md.Node>[]) {
        if (child is md.Text && child.text.trim().isEmpty) continue;
        if (child is md.Text) {
          final text = _squash(child.text);
          if (text.isNotEmpty) out.add(_stop(text));
        } else if (child is md.Element && !_blockTags.contains(child.tag) && child.tag != 'pre') {
          // Inline text sitting among blocks in a list item.
          final text = _inline(child);
          if (text.isNotEmpty) out.add(_stop(text));
        } else {
          _block(child, out);
        }
      }
      return;
  }
  final text = _inline(node);
  if (text.isEmpty) return;
  out.add(node.tag.startsWith('h') || node.tag == 'li' || node.tag == 'p' ? _stop(text) : text);
}

bool _hasBlockChild(md.Element element) =>
    element.children?.any((child) => child is md.Element && (_blockTags.contains(child.tag) || child.tag == 'pre')) ?? false;

/// The text of an inline run: code as its text, a link as its label, an image as nothing, a line break as a space.
String _inline(md.Node node) {
  final buffer = StringBuffer();
  void walk(md.Node n) {
    if (n is md.Text) {
      buffer.write(n.text);
    } else if (n is md.Element) {
      switch (n.tag) {
        case 'img' || 'pre' || 'input':
          return;
        case 'br':
          buffer.write(' ');
        default:
          n.children?.forEach(walk);
          // An item or paragraph nested in the run still ends as a sentence.
          if (!identical(n, node) && (n.tag == 'li' || n.tag == 'p')) buffer.write('. ');
      }
    }
  }

  walk(node);
  return _squash(buffer.toString());
}

String _squash(String text) => text.replaceAll(RegExp(r'\s+'), ' ').trim();

String _stop(String text) => RegExp(r'''[.!?…:;]["')\]]*$''').hasMatch(text) ? text : '$text.';

/// Finds the sentences of a reply as it grows. Each [feed] takes the whole text so far and returns the sentences that became
/// complete since the last one, once each and in order; a sentence is complete when its end is followed by more text, or when the
/// reply is [feed]'s `last`. What is read of a reply does not change when more of it arrives, so nothing is spoken twice.
final class SentenceCutter {
  SentenceCutter({this.longest = 320});

  /// The most characters one sentence may have; a longer run is cut at the last space before it.
  final int longest;

  int _taken = 0;
  bool _finished = false;

  /// Whether the last of the reply has been taken.
  bool get finished => _finished;

  List<String> feed(String markdown, {bool last = false}) {
    if (_finished) return const [];
    final prose = _prose(markdown);
    final out = <String>[];
    var start = _taken;
    if (start > prose.length) start = prose.length;
    // A boundary: a sentence's ending marks, then a space or the end of a line.
    final ends = RegExp(r'''[.!?…]+["')\]]*(?=\s)|\n''');
    var from = start;
    while (true) {
      final match = ends.allMatches(prose, from).firstOrNull;
      if (match == null) break;
      final piece = prose.substring(start, match.end).trim();
      from = match.end;
      if (piece.isNotEmpty) {
        out.addAll(_split(piece));
      }
      start = from;
    }
    // What is left may be a sentence still being written; a run that is already long is cut at a space.
    var rest = prose.substring(start);
    while (rest.trim().length > longest) {
      final cut = rest.lastIndexOf(' ', longest);
      if (cut <= 0) break;
      out.add(rest.substring(0, cut).trim());
      start += cut;
      rest = prose.substring(start);
    }
    if (last) {
      final tail = rest.trim();
      if (tail.isNotEmpty) out.add(tail);
      start = prose.length;
      _finished = true;
    }
    _taken = start;
    return out;
  }

  List<String> _split(String piece) {
    if (piece.length <= longest) return [piece];
    final parts = <String>[];
    var rest = piece;
    while (rest.length > longest) {
      final cut = rest.lastIndexOf(' ', longest);
      if (cut <= 0) break;
      parts.add(rest.substring(0, cut).trim());
      rest = rest.substring(cut).trim();
    }
    if (rest.isNotEmpty) parts.add(rest);
    return parts;
  }
}

/// A reply's sentences, all at once.
List<String> sentencesOf(String markdown) => SentenceCutter().feed(markdown, last: true);
