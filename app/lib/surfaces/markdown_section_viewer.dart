import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../theme/look.dart';
import 'files_surface.dart' show visibleText;

/// One heading-delimited section of a Markdown document.
final class MarkdownSection {
  const MarkdownSection({required this.level, required this.title, required this.slug, required this.body});

  /// Heading level, 1 for `#` through 6 for `######`.
  final int level;

  /// Heading text without the `#` marks.
  final String title;

  /// The heading's anchor, unique in the document: its words in lower case joined by hyphens, with `-2`, `-3` for a repeat.
  final String slug;

  /// The lines below the heading, up to the next heading.
  final String body;

  /// Whether [anchor] names this section: its slug, or its title in any case. A leading `#` is ignored.
  bool matchesAnchor(String anchor) {
    final bare = anchor.replaceFirst(RegExp(r'^#+'), '').trim();
    if (bare.isEmpty) return false;
    return slug == bare.toLowerCase() || slug == slugifyHeading(bare) || title.toLowerCase() == bare.toLowerCase();
  }
}

/// The frontmatter, the text before the first heading and the sections of a Markdown document.
final class ParsedMarkdownDocument {
  const ParsedMarkdownDocument({required this.frontmatter, required this.preamble, required this.sections});

  /// The top-level keys of the frontmatter, in order, each with its value as the file wrote it: a list or a nested block is its
  /// own lines, unparsed.
  final List<MapEntry<String, String>> frontmatter;
  final String preamble;
  final List<MarkdownSection> sections;

  /// Whether the sectioned view is worth showing by default: the file has frontmatter or at least three headings.
  bool get isStructured => frontmatter.isNotEmpty || sections.length >= 3;

  /// The section [anchor] names, or null.
  MarkdownSection? findSection(String? anchor) {
    if (anchor == null || anchor.trim().isEmpty) return null;
    for (final section in sections) {
      if (section.matchesAnchor(anchor)) return section;
    }
    return null;
  }
}

/// The anchor a heading's words make: lower case, runs of anything but letters and digits as one hyphen.
String slugifyHeading(String heading) => heading
    .trim()
    .toLowerCase()
    .replaceAll(RegExp(r'[`*_~]'), '')
    .replaceAll(RegExp(r'[^a-z0-9]+'), '-')
    .replaceAll(RegExp(r'^-+|-+$'), '');

final _fenceOpen = RegExp(r'^ {0,3}(`{3,}|~{3,})(.*)$');
final _heading = RegExp(r'^ {0,3}(#{1,6})[ \t]+(.*?)(?:[ \t]+#+)?[ \t]*$');
final _topKey = RegExp(r'^([A-Za-z0-9_][A-Za-z0-9_.-]*)[ \t]*:(?:[ \t]+(.*))?$');

/// Splits leading `---` frontmatter from [source]; the second value is the rest of the text.
(List<MapEntry<String, String>>, String) _splitFrontmatter(List<String> lines) {
  if (lines.isEmpty || lines.first.trimRight() != '---') return (const [], lines.join('\n'));
  final end = lines.indexWhere((line) => line.trimRight() == '---' || line.trimRight() == '...', 1);
  if (end == -1) return (const [], lines.join('\n'));
  final entries = <MapEntry<String, String>>[];
  String? key;
  var value = <String>[];
  void flush() {
    if (key != null) entries.add(MapEntry(key!, value.join('\n').trim()));
    key = null;
    value = [];
  }

  for (final line in lines.sublist(1, end)) {
    final top = line.isNotEmpty && line[0] != ' ' && line[0] != '\t' && line[0] != '-' && line[0] != '#' ? _topKey.firstMatch(line) : null;
    if (top != null) {
      flush();
      key = top.group(1);
      final inline = (top.group(2) ?? '').trim();
      if (inline.isNotEmpty) value.add(_unquoted(inline));
    } else if (key != null) {
      value.add(line);
    }
  }
  flush();
  return (entries, lines.sublist(end + 1).join('\n'));
}

String _unquoted(String value) =>
    value.length >= 2 && (value.startsWith('"') && value.endsWith('"') || value.startsWith("'") && value.endsWith("'")) ? value.substring(1, value.length - 1) : value;

/// Reads the frontmatter and the `#` to `######` sections of [source]. A heading inside a fenced code block is not one: a fence
/// closes only with the same mark, at least as long as the one that opened it.
ParsedMarkdownDocument parseMarkdownSections(String source) {
  final (frontmatter, rest) = _splitFrontmatter(source.replaceAll('\r\n', '\n').split('\n'));
  final preamble = <String>[];
  final sections = <MarkdownSection>[];
  final seen = <String, int>{};
  int? level;
  String? title;
  var body = <String>[];
  String? fence;

  void flush() {
    if (level == null || title == null) return;
    final base = slugifyHeading(title!);
    final key = base.isEmpty ? 'section' : base;
    final count = (seen[key] ?? 0) + 1;
    seen[key] = count;
    sections.add(MarkdownSection(level: level!, title: title!, slug: count == 1 ? key : '$key-$count', body: body.join('\n').trim()));
    body = [];
  }

  for (final line in rest.split('\n')) {
    if (fence != null) {
      final close = RegExp('^ {0,3}(${RegExp.escape(fence[0])}{${fence.length},})[ \\t]*\$').hasMatch(line);
      if (close) fence = null;
    } else if (_fenceOpen.firstMatch(line) case final open?) {
      final mark = open.group(1)!;
      if (mark[0] != '`' || !open.group(2)!.contains('`')) fence = mark;
    } else if (_heading.firstMatch(line) case final head?) {
      flush();
      level = head.group(1)!.length;
      title = head.group(2)!.trim();
      continue;
    }
    (level == null ? preamble : body).add(line);
  }
  flush();
  return ParsedMarkdownDocument(frontmatter: frontmatter, preamble: preamble.join('\n').trim(), sections: sections);
}

/// A Markdown document as its frontmatter, an outline and one card per section, opened on [initialSection] when it names one.
/// Nothing here edits: it shows what the file says.
class MarkdownSectionViewer extends StatefulWidget {
  const MarkdownSectionViewer({super.key, required this.text, this.initialSection});

  final String text;

  /// The slug or title of the section to focus first.
  final String? initialSection;

  @override
  State<MarkdownSectionViewer> createState() => _MarkdownSectionViewerState();
}

class _MarkdownSectionViewerState extends State<MarkdownSectionViewer> {
  late ParsedMarkdownDocument _doc;
  bool _showOutline = true;
  String? _focusedSlug;
  final Set<String> _collapsed = <String>{};
  final Map<String, GlobalKey> _keys = <String, GlobalKey>{};

  @override
  void initState() {
    super.initState();
    _parse();
  }

  @override
  void didUpdateWidget(MarkdownSectionViewer oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.text != widget.text || oldWidget.initialSection != widget.initialSection) _parse();
  }

  void _parse() {
    _doc = parseMarkdownSections(widget.text);
    _collapsed.clear();
    _keys
      ..clear()
      ..addEntries(_doc.sections.map((each) => MapEntry(each.slug, GlobalKey())));
    final target = _doc.findSection(widget.initialSection);
    _focusedSlug = target?.slug;
    if (target != null) WidgetsBinding.instance.addPostFrameCallback((_) => _scrollTo(target.slug));
  }

  void _scrollTo(String slug) {
    final context = _keys[slug]?.currentContext;
    if (context != null && context.mounted) Scrollable.ensureVisible(context, duration: const Duration(milliseconds: 180), alignment: 0.05);
  }

  void _select(MarkdownSection section) {
    setState(() {
      _focusedSlug = section.slug;
      _collapsed.remove(section.slug);
    });
    WidgetsBinding.instance.addPostFrameCallback((_) => _scrollTo(section.slug));
  }

  @override
  Widget build(BuildContext context) {
    final divider = Theme.of(context).dividerColor;
    final muted = TextStyle(color: context.chatLook.muted, fontSize: 12);
    return Column(
      key: const ValueKey('markdown section viewer'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (_doc.sections.isNotEmpty || _focusedSlug != null)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            child: Row(children: [
              if (_doc.sections.isNotEmpty)
                TextButton.icon(
                  key: const ValueKey('markdown outline toggle'),
                  icon: Icon(_showOutline ? Icons.toc : Icons.menu_open, size: 16),
                  label: Text('Outline (${_doc.sections.length})', style: const TextStyle(fontSize: 12)),
                  onPressed: () => setState(() => _showOutline = !_showOutline),
                ),
              if (_focusedSlug != null)
                TextButton.icon(
                  key: const ValueKey('markdown show all'),
                  icon: const Icon(Icons.unfold_more, size: 15),
                  label: const Text('Clear focus', style: TextStyle(fontSize: 12)),
                  onPressed: () => setState(() => _focusedSlug = null),
                ),
            ]),
          ),
        const Divider(height: 1),
        Expanded(
          child: Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            if (_showOutline && _doc.sections.isNotEmpty)
              Container(
                key: const ValueKey('markdown outline'),
                width: 170,
                decoration: BoxDecoration(border: Border(right: BorderSide(color: divider))),
                child: ListView(padding: const EdgeInsets.symmetric(vertical: 8), children: [
                  for (final section in _doc.sections)
                    InkWell(
                      key: ValueKey('markdown outline ${section.slug}'),
                      onTap: () => _select(section),
                      child: Container(
                        color: _focusedSlug == section.slug ? Theme.of(context).colorScheme.primary.withValues(alpha: 0.14) : null,
                        padding: EdgeInsets.fromLTRB(8.0 + (section.level - 1) * 8.0, 6, 8, 6),
                        child: Text(
                          visibleText(section.title),
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(fontSize: 12, fontWeight: _focusedSlug == section.slug || section.level == 1 ? FontWeight.w600 : FontWeight.w400),
                        ),
                      ),
                    ),
                ]),
              ),
            Expanded(
              child: SingleChildScrollView(
                key: const ValueKey('markdown sections scroll'),
                padding: const EdgeInsets.all(14),
                child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                  if (_doc.frontmatter.isNotEmpty) _frontmatter(context, muted),
                  if (_doc.preamble.isNotEmpty)
                    Padding(
                      padding: const EdgeInsets.only(bottom: 12),
                      child: SelectableText(visibleText(_doc.preamble), key: const ValueKey('markdown preamble'), style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12)),
                    ),
                  for (final section in _doc.sections) _card(context, section),
                ]),
              ),
            ),
          ]),
        ),
      ],
    );
  }

  Widget _frontmatter(BuildContext context, TextStyle muted) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      key: const ValueKey('markdown frontmatter'),
      margin: const EdgeInsets.only(bottom: 12),
      padding: const EdgeInsets.all(10),
      decoration: BoxDecoration(color: scheme.surfaceContainerLow, borderRadius: BorderRadius.circular(8), border: Border.all(color: Theme.of(context).dividerColor)),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        for (final entry in _doc.frontmatter)
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 2),
            child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
              SizedBox(width: 88, child: Text(visibleText(entry.key), style: muted.copyWith(fontWeight: FontWeight.w600))),
              Expanded(child: SelectableText(visibleText(entry.value), key: ValueKey('markdown frontmatter ${entry.key}'), style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12))),
            ]),
          ),
      ]),
    );
  }

  Widget _card(BuildContext context, MarkdownSection section) {
    final scheme = Theme.of(context).colorScheme;
    final focused = _focusedSlug == section.slug;
    final collapsed = _collapsed.contains(section.slug);
    final size = switch (section.level) {
      1 => 17.0,
      2 => 15.0,
      3 => 13.5,
      _ => 12.5,
    };
    return Container(
      key: _keys[section.slug],
      margin: const EdgeInsets.only(bottom: 10),
      decoration: BoxDecoration(
        color: scheme.surfaceContainerLow,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: focused ? scheme.primary : Theme.of(context).dividerColor, width: focused ? 1.5 : 1.0),
      ),
      child: Column(key: ValueKey('markdown section ${section.slug}'), crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        InkWell(
          key: ValueKey('markdown heading ${section.slug}'),
          onTap: () => setState(() => collapsed ? _collapsed.remove(section.slug) : _collapsed.add(section.slug)),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
            child: Row(children: [
              Icon(collapsed ? Icons.chevron_right : Icons.expand_more, size: 18),
              const SizedBox(width: 4),
              Expanded(child: Text(visibleText(section.title), style: TextStyle(fontSize: size, fontWeight: FontWeight.w600))),
              if (focused)
                Container(
                  key: ValueKey('markdown focused ${section.slug}'),
                  padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                  decoration: BoxDecoration(color: scheme.primary.withValues(alpha: 0.16), borderRadius: BorderRadius.circular(4)),
                  child: const Text('Focused', style: TextStyle(fontSize: 10, fontWeight: FontWeight.w600)),
                ),
            ]),
          ),
        ),
        if (!collapsed && section.body.isNotEmpty) ...[
          const Divider(height: 1),
          Padding(
            padding: const EdgeInsets.all(10),
            child: SelectableText(visibleText(section.body), key: ValueKey('markdown body ${section.slug}'), style: const TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12)),
          ),
        ],
      ]),
    );
  }
}
