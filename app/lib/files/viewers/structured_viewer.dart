import 'dart:convert';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';
import 'package:toml/toml.dart';
import 'package:yaml/yaml.dart';

import '../../l10n/app_text.dart';
import '../../theme/look.dart';
import 'file_sample.dart';
import 'file_viewer.dart';
import 'text_viewer.dart';

/// The data formats the tree reads.
enum StructuredFormat {
  json('JSON'),
  yaml('YAML'),
  toml('TOML');

  const StructuredFormat(this.label);

  final String label;

  /// The format of a file of MIME type [type], or null.
  static StructuredFormat? of(String type) => switch (type) {
    'application/json' || 'text/json' => json,
    'application/yaml' || 'application/x-yaml' || 'text/yaml' || 'text/x-yaml' => yaml,
    'application/toml' || 'text/toml' => toml,
    _ => null,
  };
}

/// [text] read as [format] into maps, lists and scalars; throws [FormatException] for what is not that format.
Object? parseStructured(String text, StructuredFormat format) {
  try {
    return switch (format) {
      StructuredFormat.json => jsonDecode(text),
      StructuredFormat.yaml => _plain(loadYaml(text)),
      StructuredFormat.toml => _plain(TomlDocument.parse(text).toMap()),
    };
  } on FormatException {
    rethrow;
  } on Object catch (error) {
    // The TOML reader's own exceptions are not FormatExceptions.
    throw FormatException('$error');
  }
}

/// A YAML or TOML value as plain maps with text keys, lists and scalars; anything else (a date) as its text.
Object? _plain(Object? value) => switch (value) {
  Map() => {for (final MapEntry(:key, :value) in value.entries) '$key': _plain(value)},
  List() => [for (final each in value) _plain(each)],
  null || String() || num() || bool() => value,
  YamlScalar() => _plain(value.value),
  _ => '$value',
};

/// One line of the tree: a key, and either a scalar or a collection that is open or shut.
final class TreeRow {
  const TreeRow({required this.id, required this.depth, required this.key, required this.value, required this.open});

  /// The row's place in the document, `a/0/b`: what an open collection is remembered by.
  final String id;
  final int depth;

  /// The map key or list index; null for the root.
  final String? key;
  final Object? value;

  /// Whether a collection is open; false for a scalar or a shut collection.
  final bool open;

  bool get isCollection => value is Map || value is List;
  int get count => switch (value) {
    final Map map => map.length,
    final List list => list.length,
    _ => 0,
  };
}

/// The rows of [root] that show: a collection's children follow it only while it is in [opened].
List<TreeRow> treeRows(Object? root, Set<String> opened) {
  final rows = <TreeRow>[];
  void walk(String id, int depth, String? key, Object? value) {
    final isOpen = opened.contains(id);
    rows.add(TreeRow(id: id, depth: depth, key: key, value: value, open: isOpen));
    if (!isOpen) return;
    String child(Object? part) => id.isEmpty ? '$part' : '$id/$part';
    switch (value) {
      case final Map map:
        for (final MapEntry(:key, :value) in map.entries) {
          walk(child(key), depth + 1, '$key', value);
        }
      case final List list:
        for (var at = 0; at < list.length; at++) {
          walk(child(at), depth + 1, '$at', list[at]);
        }
    }
  }

  walk('', 0, null, root);
  return rows;
}

/// The collections shown open when a document is first drawn: the root and the one level under it.
Set<String> firstOpened(Object? root) {
  final ids = <String>{''};
  void level(String id, Object? value) {
    final kids = switch (value) {
      final Map map => map.entries.map((e) => MapEntry('${e.key}', e.value)),
      final List list => list.indexed.map((e) => MapEntry('${e.$1}', e.$2)),
      _ => const <MapEntry<String, Object?>>[],
    };
    for (final MapEntry(:key, :value) in kids) {
      if (value is Map || value is List) ids.add(id.isEmpty ? key : '$id/$key');
    }
  }

  level('', root);
  return ids;
}

/// JSON, YAML or TOML as a tree to open and shut, a scalar coloured by what it is. A document that does not read as its format says
/// so and is shown as text instead.
class StructuredViewer extends FileViewer {
  const StructuredViewer();

  static const String viewerId = 'tree';

  @override
  String get id => viewerId;
  @override
  String get labelKey => 'files.viewer.tree';
  @override
  Set<String> get mimeTypes => const {
    'application/json',
    'text/json',
    'application/yaml',
    'application/x-yaml',
    'text/yaml',
    'text/x-yaml',
    'application/toml',
    'text/toml',
  };
  @override
  int get priority => 10;

  /// A document cut short by the preview's size is not a document, so it is read as text.
  @override
  bool canShow(FileSample sample) => sample.isText && !sample.truncated;

  @override
  Widget build(BuildContext context, FileViewRequest request) =>
      StructuredTree(key: ValueKey('tree ${request.sample.path}'), sample: request.sample, format: StructuredFormat.of(request.type) ?? StructuredFormat.json);
}

/// The tree of one file.
class StructuredTree extends StatefulWidget {
  const StructuredTree({super.key, required this.sample, required this.format});

  final FileSample sample;
  final StructuredFormat format;

  @override
  State<StructuredTree> createState() => _StructuredTreeState();
}

class _StructuredTreeState extends State<StructuredTree> {
  late Object? _root;
  String? _problem;
  late Set<String> _opened;

  @override
  void initState() {
    super.initState();
    _read();
  }

  @override
  void didUpdateWidget(StructuredTree old) {
    super.didUpdateWidget(old);
    if (old.sample != widget.sample || old.format != widget.format) _read();
  }

  void _read() {
    try {
      _root = widget.sample.memo('tree ${widget.format.name}', () => parseStructured(widget.sample.text ?? '', widget.format));
      _problem = null;
    } on FormatException catch (error) {
      _root = null;
      _problem = error.message;
    }
    _opened = _problem == null ? firstOpened(_root) : <String>{};
  }

  void _toggle(TreeRow row) => setState(() => row.open ? _opened.remove(row.id) : _opened.add(row.id));

  @override
  Widget build(BuildContext context) {
    final look = context.chatLook;
    if (_problem != null) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 12, 16, 0),
            child: Text(
              context.t('files.tree.unreadable', {'format': widget.format.label, 'reason': _problem}),
              key: const ValueKey('tree problem'),
              style: TextStyle(color: Theme.of(context).colorScheme.error, fontSize: 12),
            ),
          ),
          Expanded(child: TextLines(text: widget.sample.text ?? '', language: languageOfSample(widget.sample))),
        ],
      );
    }
    final rows = treeRows(_root, _opened);
    final mono = TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, color: look.text);
    return ListView.builder(
      key: const ValueKey('tree rows'),
      padding: const EdgeInsets.symmetric(vertical: 8, horizontal: 8),
      itemCount: rows.length,
      itemBuilder: (context, at) {
        final row = rows[at];
        final collection = row.isCollection;
        return InkWell(
          key: ValueKey('tree row ${row.id}'),
          onTap: collection ? () => _toggle(row) : null,
          child: Padding(
            padding: EdgeInsets.fromLTRB(row.depth * 16.0, 2, 4, 2),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(
                  width: 18,
                  child: collection ? Icon(row.open ? Icons.expand_more : Icons.chevron_right, size: 16, color: look.muted) : null,
                ),
                if (row.key != null) Text('${visibleText(row.key!)}: ', style: mono.copyWith(color: look.accent, fontWeight: FontWeight.w600)),
                Expanded(child: _value(context, row, mono, look)),
              ],
            ),
          ),
        );
      },
    );
  }

  Widget _value(BuildContext context, TreeRow row, TextStyle mono, ChatLook look) {
    final value = row.value;
    if (value is Map) return Text('{${row.count}}', style: mono.copyWith(color: look.muted));
    if (value is List) return Text('[${row.count}]', style: mono.copyWith(color: look.muted));
    final (text, color) = switch (value) {
      null => ('null', look.muted),
      bool() => ('$value', look.warning),
      num() => ('$value', look.accent),
      _ => (jsonEncode(visibleText('$value')), look.success),
    };
    return Text(text, key: ValueKey('tree value ${row.id}'), maxLines: 6, overflow: TextOverflow.ellipsis, style: mono.copyWith(color: color));
  }
}
