/// Draws an [A2uiSurface] with Canoa's own widgets in the colours of the look: the person's theme styles every component, and an
/// agent or an overlay chooses structure only. Nothing here fetches an address or runs what a surface carries; a button hands its
/// action to the caller, and the one function a button may call opens a web link.
library;

import 'dart:convert';

import 'package:flutter/material.dart';

import '../../theme/look.dart';
import 'messages.dart';
import 'model.dart';

/// The words the renderer says of its own, which the app gives in the person's language; an agent's own text is drawn as sent.
final class A2uiWords {
  const A2uiWords({
    this.problems = defaultProblems,
    this.waiting = defaultWaiting,
    this.moreRows = defaultMoreRows,
  });

  static const defaultProblems = 'This view has problems';
  static const defaultWaiting = 'Waiting for the view to arrive.';
  static String defaultMoreRows(int n) => '$n more rows are not shown.';

  /// The title over what is wrong with a view, the line while its root has not arrived, and how many rows a list left out.
  final String problems, waiting;
  final String Function(int count) moreRows;
}

/// What a button did, as the protocol's `action` message.
typedef A2uiActionSink = void Function(Map<String, Object?> action);

/// The icons a surface may name, by the basic catalog's names.
const Map<String, IconData> a2uiIcons = {
  'accountCircle': Icons.account_circle_outlined,
  'add': Icons.add,
  'arrowBack': Icons.arrow_back,
  'arrowForward': Icons.arrow_forward,
  'calendarToday': Icons.calendar_today_outlined,
  'call': Icons.call_outlined,
  'check': Icons.check,
  'close': Icons.close,
  'code': Icons.code,
  'contentCopy': Icons.content_copy,
  'delete': Icons.delete_outline,
  'download': Icons.download_outlined,
  'edit': Icons.edit_outlined,
  'error': Icons.error_outline,
  'favorite': Icons.favorite_border,
  'folder': Icons.folder_outlined,
  'help': Icons.help_outline,
  'home': Icons.home_outlined,
  'info': Icons.info_outline,
  'lock': Icons.lock_outline,
  'mail': Icons.mail_outline,
  'menu': Icons.menu,
  'notifications': Icons.notifications_none,
  'person': Icons.person_outline,
  'refresh': Icons.refresh,
  'search': Icons.search,
  'send': Icons.send_outlined,
  'settings': Icons.settings_outlined,
  'share': Icons.share_outlined,
  'star': Icons.star_border,
  'upload': Icons.upload_outlined,
  'visibility': Icons.visibility_outlined,
  'warning': Icons.warning_amber_outlined,
};

/// A surface, scrolled, with what was cut or wrong said above it.
class A2uiSurfaceView extends StatelessWidget {
  const A2uiSurfaceView({super.key, required this.surface, this.onAction, this.onOpenUrl, this.words = const A2uiWords(), this.padding = const EdgeInsets.all(12)});

  /// What the renderer says of its own.
  final A2uiWords words;

  final A2uiSurface surface;

  /// Where a button's event goes; null leaves buttons that raise events switched off (a view with no agent behind it).
  final A2uiActionSink? onAction;

  /// Opens a web address for the `openUrl` function; null leaves it out.
  final Future<void> Function(Uri url)? onOpenUrl;
  final EdgeInsets padding;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: surface,
    builder: (context, _) {
      final look = context.look;
      final text = Theme.of(context).textTheme;
      final notes = [...surface.cuts];
      return SelectionArea(
        child: SingleChildScrollView(
          padding: padding,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (notes.isNotEmpty)
                _Note(key: const ValueKey('a2ui cut notice'), lines: notes, colour: look.warning),
              if (surface.problems.isNotEmpty)
                _Note(
                  key: const ValueKey('a2ui problems'),
                  lines: [for (final each in surface.problems.take(5)) '${each.message}${each.path.isEmpty ? '' : ' (${each.path})'}'],
                  colour: look.danger,
                  title: words.problems,
                ),
              if (!surface.hasRoot)
                Padding(
                  padding: const EdgeInsets.all(8),
                  child: Text(words.waiting, key: const ValueKey('a2ui waiting'), style: text.bodyMedium?.copyWith(color: look.muted)),
                )
              else
                _Draw(context, surface, onAction, onOpenUrl, words).node('root', const _Scope(), 0, const {}),
            ],
          ),
        ),
      );
    },
  );
}

class _Note extends StatelessWidget {
  const _Note({super.key, required this.lines, required this.colour, this.title});
  final List<String> lines;
  final Color colour;
  final String? title;

  @override
  Widget build(BuildContext context) {
    final text = Theme.of(context).textTheme;
    return Container(
      margin: const EdgeInsets.only(bottom: 8),
      padding: const EdgeInsets.all(8),
      decoration: BoxDecoration(
        color: colour.withValues(alpha: 0.1),
        border: Border.all(color: colour.withValues(alpha: 0.6)),
        borderRadius: BorderRadius.circular(context.look.controlRadius),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (title != null) Text(title!, style: text.labelLarge?.copyWith(color: colour)),
          for (final line in lines) Text(line, style: text.bodySmall),
        ],
      ),
    );
  }
}

/// Where a component is drawn in a list template: the item and its place in the data model.
final class _Scope {
  const _Scope({this.item, this.base});
  final Object? item;
  final String? base;
}

/// One drawing of a surface: it counts what it draws against the limits.
final class _Draw {
  _Draw(this.context, this.surface, this.onAction, this.onOpenUrl, this.words);

  final A2uiWords words;

  final BuildContext context;
  final A2uiSurface surface;
  final A2uiActionSink? onAction;
  final Future<void> Function(Uri url)? onOpenUrl;
  var _drawn = 0;
  var _cutNoted = false;

  CanoaLook get look => context.look;
  TextTheme get text => Theme.of(context).textTheme;

  Widget notice(String line) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
    decoration: BoxDecoration(border: Border.all(color: look.danger.withValues(alpha: 0.6)), borderRadius: BorderRadius.circular(look.controlRadius)),
    child: Text(line, key: const ValueKey('a2ui component notice'), style: text.bodySmall?.copyWith(color: look.danger)),
  );

  // ---- values

  (bool, Object?) _lookup(String path, _Scope scope) {
    if (path.startsWith('/')) return pointerRead(surface.data, path);
    if (scope.item == null) return (false, null);
    return pointerRead(scope.item, '/$path');
  }

  String? _absolute(String path, _Scope scope) {
    if (path.startsWith('/')) return path;
    final base = scope.base;
    return base == null ? null : '$base/$path';
  }

  Object? eval(Object? value, _Scope scope) {
    if (value is Map) {
      if (value['@path'] is String) return _lookup(value['@path'] as String, scope).$2;
      if (value['@call'] is String) return _call(value['@call'] as String, value['args'], scope);
      return value;
    }
    return value;
  }

  String str(Object? value, _Scope scope) {
    final v = eval(value, scope);
    final out = switch (v) {
      null => '',
      String() => v,
      double() when v == v.roundToDouble() && v.abs() < 1e15 => v.toInt().toString(),
      num() || bool() => '$v',
      _ => jsonEncode(v),
    };
    final limit = surface.limits.maxText;
    return out.length > limit ? '${out.substring(0, limit)}…' : out;
  }

  Object? _call(String name, Object? rawArgs, _Scope scope) {
    final args = rawArgs is Map ? {for (final entry in rawArgs.entries) '${entry.key}': eval(entry.value, scope)} : <String, Object?>{};
    switch (name) {
      case 'formatString':
        final template = '${args['value'] ?? ''}'.replaceAll(r'\${', '\u0000');
        return template
            .replaceAllMapped(RegExp(r'\$\{([^}]*)\}'), (match) {
              final found = _lookup(match.group(1)!.trim(), scope);
              return found.$1 ? str(found.$2, scope) : '';
            })
            .replaceAll('\u0000', r'${');
      case 'formatNumber':
        final number = args['value'];
        if (number is! num) return '';
        final decimals = args['decimals'] is num ? (args['decimals'] as num).toInt().clamp(0, 12) : null;
        var out = decimals == null ? (number == number.roundToDouble() ? '${number.toInt()}' : '$number') : number.toStringAsFixed(decimals);
        if (args['grouping'] != false) {
          final parts = out.split('.');
          final sign = parts.first.startsWith('-') ? '-' : '';
          final digits = parts.first.replaceFirst('-', '');
          final grouped = digits.replaceAllMapped(RegExp(r'\B(?=(\d{3})+(?!\d))'), (_) => ',');
          out = '$sign$grouped${parts.length > 1 ? '.${parts[1]}' : ''}';
        }
        return out;
      case 'pluralize':
        final count = args['value'];
        if (count is! num) return args['other'];
        if (count == 0 && args['zero'] != null) return args['zero'];
        if (count == 1 && args['one'] != null) return args['one'];
        if (count == 2 && args['two'] != null) return args['two'];
        return args['other'];
      case 'required':
        final v = args['value'];
        return v != null && v != '' && !(v is List && v.isEmpty);
      case 'and':
        return args['values'] is List && (args['values'] as List).every((each) => each == true);
      case 'or':
        return args['values'] is List && (args['values'] as List).any((each) => each == true);
      case 'not':
        return args['value'] != true;
    }
    return null;
  }

  // ---- tree

  Widget node(String id, _Scope scope, int depth, Set<String> path) {
    final limits = surface.limits;
    if (++_drawn > limits.maxNodes) {
      if (_cutNoted) return const SizedBox.shrink();
      _cutNoted = true;
      return notice('More than ${limits.maxNodes} components: the rest are not drawn.');
    }
    if (depth > limits.maxDepth) return notice('Deeper than ${limits.maxDepth} levels: not drawn.');
    if (path.contains(id)) return notice('"$id" holds itself: not drawn again.');
    final raw = surface.components[id];
    if (raw == null) return notice('No component "$id".');
    final name = '${raw['component']}';
    final spec = surface.catalog[name];
    if (spec == null) return notice('"$name" is not a component of this catalog.');
    final props = {...spec.defaults, ...raw};
    final here = {...path, id};
    Widget child(Object? ref) => ref is String ? node(ref, scope, depth + 1, here) : const SizedBox.shrink();
    final built = _native(spec.native, id, props, scope, depth, here, child);
    return KeyedSubtree(key: ValueKey('a2ui $id${scope.base ?? ''}'), child: built);
  }

  /// The widgets of a `children` value: a list of ids, or a template over a list in the data model, cut at the row limit.
  List<Widget> children(Object? value, _Scope scope, int depth, Set<String> path) {
    if (value is List) return [for (final each in value) if (each is String) node(each, scope, depth + 1, path)];
    if (value is Map && value['componentId'] is String && value['path'] is String) {
      final source = value['path'] as String;
      final absolute = _absolute(source, scope);
      final found = _lookup(source, scope).$2;
      if (absolute == null || found is! List) return const [];
      final rows = found.length > surface.limits.maxRows ? surface.limits.maxRows : found.length;
      return [
        for (var i = 0; i < rows; i++) node(value['componentId'] as String, _Scope(item: found[i], base: '$absolute/$i'), depth + 1, path),
        if (found.length > rows)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(words.moreRows(found.length - rows), key: const ValueKey('a2ui rows cut'), style: text.bodySmall?.copyWith(color: look.muted)),
          ),
      ];
    }
    return const [];
  }

  MainAxisAlignment _main(Object? v) => switch (v) {
    'center' => MainAxisAlignment.center,
    'end' => MainAxisAlignment.end,
    'spaceBetween' => MainAxisAlignment.spaceBetween,
    'spaceAround' => MainAxisAlignment.spaceAround,
    'spaceEvenly' => MainAxisAlignment.spaceEvenly,
    _ => MainAxisAlignment.start,
  };

  CrossAxisAlignment _cross(Object? v) => switch (v) {
    'center' => CrossAxisAlignment.center,
    'end' => CrossAxisAlignment.end,
    'start' => CrossAxisAlignment.start,
    _ => CrossAxisAlignment.stretch,
  };

  Color _tone(Object? tone) => switch (tone) {
    'success' => look.success,
    'warning' => look.warning,
    'danger' => look.danger,
    _ => look.accent,
  };

  Widget _native(String native, String id, Map<String, Object?> p, _Scope scope, int depth, Set<String> path, Widget Function(Object?) child) {
    switch (native) {
      case 'Text':
        final caption = p['variant'] == 'caption';
        return Text(str(p['text'], scope), style: caption ? text.bodySmall?.copyWith(color: look.muted) : text.bodyMedium);
      case 'Column':
        return Column(
          mainAxisSize: MainAxisSize.min,
          mainAxisAlignment: _main(p['justify']),
          crossAxisAlignment: _cross(p['align']),
          children: [for (final each in _spaced(children(p['children'], scope, depth, path))) each],
        );
      case 'Row':
        final ids = p['children'];
        final built = children(ids, scope, depth, path);
        return Row(
          mainAxisAlignment: _main(p['justify']),
          crossAxisAlignment: _cross(p['align']) == CrossAxisAlignment.stretch ? CrossAxisAlignment.center : _cross(p['align']),
          children: [
            for (var i = 0; i < built.length; i++)
              Flexible(flex: _weight(ids, i), child: built[i]),
          ],
        );
      case 'List':
        final built = children(p['children'], scope, depth, path);
        if (p['direction'] == 'horizontal') {
          return SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [for (final each in built) Padding(padding: const EdgeInsets.only(right: 8), child: ConstrainedBox(constraints: const BoxConstraints(maxWidth: 320), child: each))]),
          );
        }
        return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: _cross(p['align']), children: _spaced(built));
      case 'Card':
        return Card(margin: EdgeInsets.zero, child: Padding(padding: const EdgeInsets.all(12), child: child(p['child'])));
      case 'Divider':
        return p['axis'] == 'vertical'
            ? Container(width: 1, height: 20, margin: const EdgeInsets.symmetric(horizontal: 6), color: look.border)
            : Divider(height: 16, color: look.border);
      case 'Icon':
        final name = p['name'];
        return Icon(name is String ? (a2uiIcons[name] ?? Icons.help_outline) : Icons.help_outline, size: 20, color: look.muted);
      case 'Image':
        return _image(p, scope);
      case 'Tabs':
        final tabs = [if (p['tabs'] is List) for (final each in p['tabs'] as List) if (each is Map && each['child'] is String) each];
        return _A2uiTabs(
          key: ValueKey('a2ui tabs $id${scope.base ?? ''}'),
          titles: [for (final tab in tabs) str(tab['title'], scope)],
          body: (index) => node(tabs[index]['child'] as String, scope, depth + 1, path),
        );
      case 'Modal':
        return _Modal(trigger: child(p['trigger']), content: () => node('${p['content']}', scope, depth + 1, path));
      case 'Button':
        return _button(id, p, scope, child(p['child']));
      case 'TextField':
        final bound = p['value'] is Map && (p['value'] as Map)['@path'] is String ? _absolute((p['value'] as Map)['@path'] as String, scope) : null;
        final variant = p['variant'];
        return _BoundField(
          key: ValueKey('a2ui field $id${scope.base ?? ''}'),
          label: str(p['label'], scope),
          value: str(p['value'], scope),
          multiline: variant == 'longText',
          number: variant == 'number',
          obscured: variant == 'obscured',
          onChanged: bound == null ? null : (value) => surface.putData(bound, value),
        );
      case 'CheckBox':
        final bound = p['value'] is Map && (p['value'] as Map)['@path'] is String ? _absolute((p['value'] as Map)['@path'] as String, scope) : null;
        return CheckboxListTile(
          dense: true,
          contentPadding: EdgeInsets.zero,
          controlAffinity: ListTileControlAffinity.leading,
          title: Text(str(p['label'], scope)),
          value: eval(p['value'], scope) == true,
          onChanged: bound == null ? null : (value) => surface.putData(bound, value ?? false),
        );
      case 'Badge':
        final tone = _tone(p['tone']);
        return Align(
          alignment: Alignment.centerLeft,
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
            decoration: BoxDecoration(color: tone.withValues(alpha: 0.15), borderRadius: BorderRadius.circular(look.controlRadius), border: Border.all(color: tone.withValues(alpha: 0.5))),
            child: Text(str(p['text'], scope), style: text.labelMedium?.copyWith(color: tone)),
          ),
        );
      case 'Metric':
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(str(p['label'], scope), style: text.bodySmall?.copyWith(color: look.muted)),
            Text(str(p['value'], scope), style: text.headlineSmall?.copyWith(color: look.text)),
            if (p['caption'] != null) Text(str(p['caption'], scope), style: text.bodySmall?.copyWith(color: look.muted)),
          ],
        );
      case 'Progress':
        final raw = eval(p['value'], scope);
        final fraction = raw is num ? raw.toDouble().clamp(0.0, 1.0) : null;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            if (p['label'] != null) Text(str(p['label'], scope), style: text.bodySmall),
            const SizedBox(height: 4),
            LinearProgressIndicator(value: fraction, color: look.accent, backgroundColor: look.border),
          ],
        );
      case 'CodeBlock':
        return Container(
          width: double.infinity,
          padding: const EdgeInsets.all(8),
          decoration: BoxDecoration(color: look.raised, border: Border.all(color: look.border), borderRadius: BorderRadius.circular(look.controlRadius)),
          child: Text(str(p['text'], scope), style: TextStyle(fontFamily: CanoaLook.monospaceFont, fontSize: 12, height: 1.35, color: look.text)),
        );
      case 'Table':
        return _table(p, scope, depth);
    }
    return notice('"$native" is not drawn here.');
  }

  List<Widget> _spaced(List<Widget> list) => [
    for (var i = 0; i < list.length; i++) ...[if (i > 0) const SizedBox(height: 8), list[i]],
  ];

  int _weight(Object? ids, int index) {
    if (ids is! List || index >= ids.length || ids[index] is! String) return 1;
    final w = surface.components[ids[index]]?['weight'];
    return w is num && w >= 1 ? w.toInt() : 1;
  }

  Widget _image(Map<String, Object?> p, _Scope scope) {
    final url = str(p['url'], scope);
    final description = p['description'] == null ? '' : str(p['description'], scope);
    final size = switch (p['variant']) {
      'icon' => 24.0,
      'avatar' => 40.0,
      'smallFeature' => 64.0,
      'largeFeature' => 200.0,
      'header' => 140.0,
      _ => 120.0,
    };
    // Only an image the message carries is drawn: nothing is fetched for a surface.
    if (url.startsWith('data:image/')) {
      try {
        final bytes = UriData.parse(url).contentAsBytes();
        if (bytes.length <= 2 * 1024 * 1024) {
          return Image.memory(
            bytes,
            height: size,
            semanticLabel: description.isEmpty ? null : description,
            fit: switch (p['fit']) {
              'cover' => BoxFit.cover,
              'fill' => BoxFit.fill,
              'none' => BoxFit.none,
              'scaleDown' => BoxFit.scaleDown,
              _ => BoxFit.contain,
            },
            errorBuilder: (_, _, _) => _imagePlaceholder(description, size),
          );
        }
      } on Object {
        // A picture that cannot be read is the placeholder.
      }
    }
    return _imagePlaceholder(description, size);
  }

  Widget _imagePlaceholder(String description, double size) => Container(
    key: const ValueKey('a2ui image placeholder'),
    height: size,
    padding: const EdgeInsets.all(6),
    decoration: BoxDecoration(color: look.raised, border: Border.all(color: look.border), borderRadius: BorderRadius.circular(look.controlRadius)),
    child: Row(mainAxisSize: MainAxisSize.min, children: [
      Icon(Icons.image_outlined, color: look.muted, size: size < 40 ? 16 : 24),
      if (description.isNotEmpty && size >= 40) Flexible(child: Padding(padding: const EdgeInsets.only(left: 6), child: Text(description, style: text.bodySmall?.copyWith(color: look.muted)))),
    ]),
  );

  Widget _button(String id, Map<String, Object?> p, _Scope scope, Widget label) {
    final action = p['action'];
    VoidCallback? press;
    if (action is Map && action['event'] is Map && onAction != null) {
      final event = action['event'] as Map;
      press = () {
        final context = <String, Object?>{
          if (event['context'] is Map)
            for (final entry in (event['context'] as Map).entries) '${entry.key}': _plain(eval(entry.value, scope)),
        };
        onAction!({
          'version': a2uiVersion,
          'action': {
            'name': '${event['name']}',
            'surfaceId': surface.id,
            'sourceComponentId': id,
            'timestamp': DateTime.now().toUtc().toIso8601String(),
            'context': context,
            if (surface.sendDataModel) 'metadata': {'dataModel': surface.dataSnapshot},
          },
        });
      };
    } else if (action is Map && action['functionCall'] is Map && onOpenUrl != null) {
      final call = action['functionCall'] as Map;
      final name = call['call'] ?? call['@call'];
      if (name == 'openUrl') {
        press = () {
          final args = call['args'] is Map ? call['args'] as Map : const {};
          final uri = Uri.tryParse(str(args['url'], scope));
          if (uri != null && (uri.scheme == 'https' || uri.scheme == 'http') && uri.host.isNotEmpty) onOpenUrl!(uri);
        };
      }
    }
    final variant = p['variant'];
    return Align(
      alignment: Alignment.centerLeft,
      child: switch (variant) {
        'primary' => FilledButton(key: ValueKey('a2ui button $id'), onPressed: press, child: label),
        'borderless' => TextButton(key: ValueKey('a2ui button $id'), onPressed: press, child: label),
        _ => OutlinedButton(key: ValueKey('a2ui button $id'), onPressed: press, child: label),
      },
    );
  }

  /// A value for the context of an action: JSON only.
  Object? _plain(Object? value) {
    try {
      return jsonDecode(jsonEncode(value));
    } on Object {
      return '$value';
    }
  }

  Widget _table(Map<String, Object?> p, _Scope scope, int depth) {
    final columns = [if (p['columns'] is List) for (final each in p['columns'] as List) if (each is Map) each];
    final rowsRef = p['rows'];
    final source = rowsRef is Map && rowsRef['path'] is String ? rowsRef['path'] as String : null;
    final data = source == null ? null : _lookup(source, scope).$2;
    final rows = data is List ? data : const [];
    final shown = rows.length > surface.limits.maxRows ? surface.limits.maxRows : rows.length;
    if (columns.isEmpty) return notice('A table needs columns.');
    TableCell cell(Widget child, {bool head = false}) => TableCell(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        decoration: BoxDecoration(color: head ? look.raised : null, border: Border(bottom: BorderSide(color: look.border))),
        child: child,
      ),
    );
    return Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
      SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Table(
          defaultColumnWidth: const IntrinsicColumnWidth(),
          children: [
            TableRow(children: [for (final c in columns) cell(Text(str(c['title'], scope), style: text.labelLarge), head: true)]),
            for (var i = 0; i < shown; i++)
              TableRow(children: [
                for (final c in columns)
                  cell(Text(c['path'] is String ? str(pointerRead(rows[i], '/${c['path']}').$2, scope) : '', style: text.bodyMedium)),
              ]),
          ],
        ),
      ),
      if (rows.length > shown) Padding(padding: const EdgeInsets.only(top: 4), child: Text(words.moreRows(rows.length - shown), key: const ValueKey('a2ui rows cut'), style: text.bodySmall?.copyWith(color: look.muted))),
    ]);
  }
}

class _A2uiTabs extends StatefulWidget {
  const _A2uiTabs({super.key, required this.titles, required this.body});
  final List<String> titles;
  final Widget Function(int index) body;

  @override
  State<_A2uiTabs> createState() => _A2uiTabsState();
}

class _A2uiTabsState extends State<_A2uiTabs> {
  var _index = 0;

  @override
  Widget build(BuildContext context) {
    final look = context.look;
    if (widget.titles.isEmpty) return const SizedBox.shrink();
    final at = _index.clamp(0, widget.titles.length - 1);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
      SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Row(children: [
          for (var i = 0; i < widget.titles.length; i++)
            InkWell(
              key: ValueKey('a2ui tab $i'),
              onTap: () => setState(() => _index = i),
              child: Container(
                padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                decoration: BoxDecoration(border: Border(bottom: BorderSide(width: 2, color: i == at ? look.accent : Colors.transparent))),
                child: Text(widget.titles[i], style: TextStyle(color: i == at ? look.text : look.muted, fontWeight: i == at ? FontWeight.w600 : FontWeight.normal)),
              ),
            ),
        ]),
      ),
      Divider(height: 1, color: look.border),
      const SizedBox(height: 8),
      widget.body(at),
    ]);
  }
}

class _Modal extends StatelessWidget {
  const _Modal({required this.trigger, required this.content});
  final Widget trigger;
  final Widget Function() content;

  @override
  Widget build(BuildContext context) => InkWell(
    key: const ValueKey('a2ui modal trigger'),
    onTap: () => showDialog<void>(
      context: context,
      builder: (_) => Dialog(child: Padding(padding: const EdgeInsets.all(16), child: SingleChildScrollView(child: Builder(builder: (_) => content())))),
    ),
    child: IgnorePointer(child: trigger),
  );
}

class _BoundField extends StatefulWidget {
  const _BoundField({super.key, required this.label, required this.value, required this.multiline, required this.number, required this.obscured, this.onChanged});
  final String label, value;
  final bool multiline, number, obscured;
  final ValueChanged<String>? onChanged;

  @override
  State<_BoundField> createState() => _BoundFieldState();
}

class _BoundFieldState extends State<_BoundField> {
  late final _controller = TextEditingController(text: widget.value);

  @override
  void didUpdateWidget(_BoundField old) {
    super.didUpdateWidget(old);
    if (widget.value != _controller.text) {
      _controller.value = TextEditingValue(text: widget.value, selection: TextSelection.collapsed(offset: widget.value.length));
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => TextField(
    controller: _controller,
    decoration: InputDecoration(labelText: widget.label, isDense: true),
    minLines: widget.multiline ? 3 : 1,
    maxLines: widget.multiline ? 8 : 1,
    keyboardType: widget.number ? TextInputType.number : null,
    obscureText: widget.obscured,
    onChanged: widget.onChanged,
  );
}
