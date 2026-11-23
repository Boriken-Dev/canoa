import 'package:coqui_acp_chat/coqui_acp_chat.dart' show ToolCall, ToolDiff, ToolPatch, ToolText;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart' show SelectedContent;
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:markdown/markdown.dart' as md;

import '../l10n/app_text.dart';
import 'syntax.dart';
import 'syntax_theme.dart';

/// The longest block drawn in one go (a chat message's code block, a tool's output): past it the block is plain, since it is lexed
/// when it is built. A file preview has no such limit: it lexes the lines in view.
const int eagerLimit = 128 * 1024;

/// How many lines apart the lexer's carried state is kept, so reaching a line far down lexes at most this many before it.
const int _stride = 64;

/// The lines of one text, lexed as they are asked for: line [at] is lexed from the nearest checkpoint before it, and the state at
/// each checkpoint is kept, so scrolling through a long file does the work of the lines drawn, once.
final class HighlightedLines {
  HighlightedLines(this.lines, this.language, {int maxChars = SyntaxTheme.defaultMaxChars})
    : _on = language != null && lines.fold<int>(0, (sum, line) => sum + line.length) <= maxChars;

  final List<String> lines;
  final Language? language;
  final bool _on;

  /// The state before line `k * _stride`, for the checkpoints reached so far.
  final List<LexState> _checkpoints = [0];

  /// Whether this text is coloured at all.
  bool get highlighted => _on;

  /// How many lines were lexed so far: what a test reads to see that scrolling did not lex the whole file.
  int lexed = 0;

  /// The spans of line [at].
  List<Span> spansAt(int at) {
    final language = this.language;
    if (!_on || language == null || at < 0 || at >= lines.length) return const [];
    var k = at ~/ _stride;
    // Reach the checkpoint of line `at`'s block, lexing on from the last one known.
    while (_checkpoints.length <= k) {
      final from = (_checkpoints.length - 1) * _stride;
      var state = _checkpoints.last;
      for (var i = from; i < from + _stride; i++) {
        state = lexLine(language, lines[i], state).$2;
        lexed++;
      }
      _checkpoints.add(state);
    }
    var state = _checkpoints[k];
    for (var i = k * _stride; i < at; i++) {
      state = lexLine(language, lines[i], state).$2;
      lexed++;
    }
    lexed++;
    return lexLine(language, lines[at], state).$1;
  }
}

/// [line] as a text span, each of its [spans] in the style of its kind.
TextSpan lineSpan(String line, List<Span> spans, Map<Tok, TextStyle> styles) {
  if (spans.isEmpty) return TextSpan(text: line, style: styles[Tok.plain]);
  final children = <TextSpan>[];
  var at = 0;
  for (final span in spans) {
    if (span.start > at) children.add(TextSpan(text: line.substring(at, span.start)));
    children.add(TextSpan(text: line.substring(span.start, span.end), style: styles[span.tok]));
    at = span.end;
  }
  if (at < line.length) children.add(TextSpan(text: line.substring(at)));
  return TextSpan(style: styles[Tok.plain], children: children);
}

/// The package's code-line hook for [look]: a line of a changed file, a permission's diff or a tool's patch is coloured as the
/// language of [path] says, and left plain with no path, an unknown language or a line past [eagerLimit] (or the theme's cap).
CodeSpans codeSpansFor(ChatLook look) => (line, path, style) {
  final language = languageForPath(path);
  if (language == null || line.length > eagerLimit.clamp(0, SyntaxTheme.defaultMaxChars)) return TextSpan(text: line, style: style);
  return lineSpan(line, lexLine(language, line).$1, const SyntaxTheme().styles(look, style));
};

final Map<String, List<List<Span>>> _recent = {};

/// The spans of every line of [code] in [language], kept for the last few blocks so a rebuild does not lex them again.
List<List<Span>> _lexAll(Language language, List<String> lines, String code) {
  final key = '${language.id}\u0000$code';
  final kept = _recent.remove(key);
  if (kept != null) {
    _recent[key] = kept;
    return kept;
  }
  var state = 0;
  final all = <List<Span>>[];
  for (final line in lines) {
    final (spans, next) = lexLine(language, line, state);
    all.add(spans);
    state = next;
  }
  _recent[key] = all;
  if (_recent.length > 48) _recent.remove(_recent.keys.first);
  return all;
}

/// [code] as one text span coloured as [language], or plain when it is too long to lex in one go or [language] is null.
TextSpan codeSpan(BuildContext context, String code, Language? language, TextStyle base, {int? limit}) {
  final theme = SyntaxTheme.of(context);
  final styles = theme.styles(context.chatLook, base);
  if (language == null || code.length > (limit ?? eagerLimit).clamp(0, theme.maxChars)) return TextSpan(text: code, style: base);
  final lines = code.split('\n');
  final all = _lexAll(language, lines, code);
  return TextSpan(style: base, children: [
    for (var i = 0; i < lines.length; i++) ...[
      if (i > 0) const TextSpan(text: '\n'),
      lineSpan(lines[i], all[i], styles),
    ],
  ]);
}

/// A selection that copies each line it covers on a line of its own, for a block built of one widget per line.
class LineBlock extends StatefulWidget {
  const LineBlock({super.key, required this.child});

  final Widget child;

  @override
  State<LineBlock> createState() => _LineBlockState();
}

class _LineBlockState extends State<LineBlock> {
  final _delegate = _LineDelegate();

  @override
  void dispose() {
    _delegate.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => SelectionContainer(delegate: _delegate, child: widget.child);
}

class _LineDelegate extends StaticSelectionContainerDelegate {
  @override
  SelectedContent? getSelectedContent() {
    final parts = <String>[
      for (final selectable in selectables)
        if (selectable.getSelectedContent() case final SelectedContent content) content.plainText,
    ];
    return parts.isEmpty ? null : SelectedContent(plainText: parts.join('\n'));
  }
}

/// A code block of a message: [code] coloured as the language [info] names (a fence's info string) or, with none, as it looks, with
/// a button that copies the code alone.
class CodeBlock extends StatelessWidget {
  const CodeBlock({super.key, required this.code, this.info, this.language});

  final String code;
  final String? info;

  /// The language when it is known already; [info] and the code itself say it otherwise.
  final Language? language;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final named = info?.trim().isNotEmpty ?? false;
    final language = this.language ?? (named ? languageForInfo(info) : sniffLanguage(code));
    final base = TextStyle(fontFamily: look.codeFont, fontSize: look.bodySize - 1, height: 1.35, color: look.text);
    return Container(
      width: double.infinity,
      margin: const EdgeInsets.symmetric(vertical: 4),
      decoration: BoxDecoration(color: look.code, borderRadius: BorderRadius.circular(look.radius / 2)),
      child: Stack(children: [
        SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          padding: const EdgeInsets.fromLTRB(10, 10, 40, 10),
          child: Text.rich(key: const ValueKey('code block text'), codeSpan(context, code, language, base)),
        ),
        Positioned(top: 0, right: 0, child: CopyButton(text: code, tooltip: context.t('code.copy'))),
      ]),
    );
  }
}

/// A tool's output as a box of at most 320 px that scrolls, coloured as [language].
class CodeOutput extends StatelessWidget {
  const CodeOutput({super.key, required this.text, required this.language});

  final String text;
  final Language? language;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final base = TextStyle(fontFamily: look.codeFont, fontSize: 12, height: 1.35, color: look.text);
    return Container(
      constraints: const BoxConstraints(maxHeight: 320),
      width: double.infinity,
      padding: const EdgeInsets.all(8),
      decoration: BoxDecoration(color: look.code, borderRadius: BorderRadius.circular(look.radius / 2)),
      child: SingleChildScrollView(child: Text.rich(codeSpan(context, text, language, base))),
    );
  }
}

/// Lines of a change as the package draws them, an added one on the look's `added` and a removed one on its `removed`, with the code
/// of each coloured as [language]. Each line is lexed alone, so a comment or string that runs over lines is coloured only where it
/// starts and ends on the line.
class HighlightedDiffView extends StatelessWidget {
  const HighlightedDiffView({super.key, required this.lines, this.language});

  /// Null marks lines left out.
  final List<DiffLine?> lines;
  final Language? language;

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    final base = TextStyle(fontFamily: look.codeFont, fontSize: 12, height: 1.35, color: look.text);
    final styles = SyntaxTheme.of(context).styles(look, base);
    final language = this.language;
    final on = language != null && lines.length <= 4000;
    return Container(
      decoration: BoxDecoration(color: look.code, borderRadius: BorderRadius.circular(look.radius / 2)),
      // One scroll direction only: a scrollable inside a scrollable under the transcript's selection makes a drag across it fail.
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: LineBlock(child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final line in lines)
              if (line == null)
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  child: Text('⋯', style: base.copyWith(color: look.muted)),
                )
              else
                Container(
                  color: switch (line.kind) {
                    DiffKind.added => look.added,
                    DiffKind.removed => look.removed,
                    DiffKind.same => null,
                  },
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  child: Text.rich(
                    TextSpan(style: base, children: [
                      TextSpan(text: '${switch (line.kind) { DiffKind.added => '+', DiffKind.removed => '-', DiffKind.same => ' ' }} '),
                      lineSpan(line.text, on ? lexLine(language, line.text).$1 : const [], styles),
                    ]),
                    softWrap: false,
                  ),
                ),
          ],
        )),
      ),
    );
  }
}

/// Draws a fenced code block of a message with [CodeBlock]; the package's own is replaced for the tag `pre`.
final class HighlightedCodeBuilder extends MarkdownElementBuilder {
  HighlightedCodeBuilder();

  @override
  bool isBlockElement() => true;

  @override
  Widget? visitElementAfterWithContext(BuildContext context, md.Element element, TextStyle? preferredStyle, TextStyle? parentStyle) {
    var code = element.textContent;
    if (code.endsWith('\n')) code = code.substring(0, code.length - 1);
    final first = element.children?.firstOrNull;
    final info = first is md.Element ? first.attributes['class'] : null;
    return CodeBlock(code: code, info: info);
  }
}

/// The element builders every [AgentMarkdown] of the app is given.
final Map<String, MarkdownElementBuilder> highlightedMarkdown = {'pre': HighlightedCodeBuilder()};

/// The path of a tool call's location, without a line number.
String? _locationPath(String place) {
  final cut = RegExp(r':\d+(:\d+)?$').firstMatch(place);
  return cut == null ? place : place.substring(0, cut.start);
}

/// The language of a unified patch's first `+++ ` file header, by the file's name.
Language? languageOfPatch(String patch) {
  final head = patch.length > 4000 ? patch.substring(0, 4000) : patch;
  final header = RegExp(r'^\+\+\+ (?:b/)?([^\t\r\n]+)', multiLine: true).firstMatch(head);
  return header == null ? null : languageForPath(header.group(1));
}

/// Colours the pieces of a tool call's content that are code: a text read from a file (its language by the file's name, else as it
/// looks), the new text of an edit, and a patch. A piece nothing is known of is left to the next renderer.
final ChatRenderers highlightRenderers = ChatRenderers(
  block: [
    (context, ToolCall tool, block) {
      switch (block) {
        case ToolText(:final text):
          Language? language;
          for (final place in tool.locations) {
            language = languageForPath(_locationPath(place));
            if (language != null) break;
          }
          language ??= sniffLanguage(text);
          return language == null ? null : CodeOutput(text: text, language: language);
        case ToolDiff(:final path, :final oldText, :final newText):
          final language = languageForPath(path);
          if (language == null) return null;
          return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Text(path, style: TextStyle(fontFamily: context.chatLook.codeFont, fontSize: 12, color: context.chatLook.muted)),
            const SizedBox(height: 4),
            HighlightedDiffView(lines: withContext(diffLines(oldText, newText)), language: language),
          ]);
        case ToolPatch(:final text):
          final language = languageOfPatch(text);
          return language == null ? null : HighlightedDiffView(lines: patchLines(text), language: language);
        default:
          return null;
      }
    },
  ],
);
