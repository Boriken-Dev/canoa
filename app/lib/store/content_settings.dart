/// Settings, Content: the URI schemes the host reads and who brought each, the catalog generators, the providers a library offered
/// that were refused, the content cache, and a Try box that asks the host about one URI. Everything is the host's answer to
/// `content/schemes`, `content/cache`, `content/stat`, `content/list`, `content/read` and `content/resolve`: the app fetches nothing.
library;

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../desktop/control_client.dart' show ControlFailed;
import '../l10n/app_text.dart';
import 'overlay_page.dart' show oneLine;

/// What the app asks the host with.
typedef ContentCall = Future<Object?> Function(String method, Map<String, Object?> params);

/// One URI scheme of the host's table.
final class ContentScheme {
  const ContentScheme({required this.scheme, required this.grammar, required this.provider, this.extension, this.lists = false, this.resolves = false, this.on = true});

  factory ContentScheme.fromAnswer(Map<Object?, Object?> each) => ContentScheme(
    scheme: '${each['scheme']}',
    grammar: '${each['grammar'] ?? ''}',
    provider: '${each['provider']}',
    extension: each['extension'] as String?,
    lists: each['lists'] == true,
    resolves: each['resolves'] == true,
    on: each['state'] != 'off',
  );

  final String scheme, grammar, provider;

  /// The extension that brought it, none for a scheme built in.
  final String? extension;
  final bool lists, resolves, on;
}

/// A catalog generator or a provider that was refused, as the host lists it.
final class ContentProvider {
  const ContentProvider({required this.id, required this.title, this.extension, this.why});

  final String id, title;
  final String? extension;

  /// Why it is not offered, for a refused one.
  final String? why;
}

/// What the content cache keeps.
final class ContentCacheState {
  const ContentCacheState({required this.entries, required this.bytes, required this.maxBytes});

  final int entries, bytes, maxBytes;
}

/// The host's table of schemes, generators and cache, and the questions of the Try box.
class ContentSettings extends ChangeNotifier {
  ContentSettings(this._call);

  final ContentCall _call;

  bool _disposed = false;
  bool _loading = false, _loaded = false;
  List<ContentScheme> _schemes = const [];
  List<ContentProvider> _generators = const [], _refused = const [];
  ContentCacheState? _cache;
  String? _problem;

  List<ContentScheme> get schemes => _schemes;
  List<ContentProvider> get generators => _generators;
  List<ContentProvider> get refused => _refused;
  ContentCacheState? get cache => _cache;
  String? get problem => _problem;
  bool get loading => _loading;
  bool get loaded => _loaded;

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  Map<Object?, Object?> _map(Object? answer) => answer is Map<Object?, Object?> ? answer : const {};

  /// Reads the table and the cache.
  Future<void> load() async {
    _loading = true;
    notifyListeners();
    try {
      final table = _map(await _call('content/schemes', const {}));
      _schemes = [for (final each in table['schemes'] as List<Object?>? ?? const []) if (each is Map<Object?, Object?>) ContentScheme.fromAnswer(each)];
      _generators = [
        for (final each in table['generators'] as List<Object?>? ?? const [])
          if (each is Map<Object?, Object?>) ContentProvider(id: _word(each['id']), title: _word(each['title'] ?? each['id']), extension: each['extension'] as String?),
      ];
      _refused = [
        for (final each in table['refused'] as List<Object?>? ?? const [])
          if (each is Map<Object?, Object?>) ContentProvider(id: _word(each['id']), title: _word(each['id']), extension: each['extension'] as String?, why: _word(each['why'])),
      ];
      await _readCache();
      _problem = null;
      _loaded = true;
    } on ControlFailed catch (error) {
      _problem = error.message;
    } on Object catch (error) {
      _problem = '$error';
    }
    _loading = false;
    notifyListeners();
  }

  Future<void> _readCache({bool clear = false}) async {
    final answer = _map(await _call('content/cache', {if (clear) 'clear': true}));
    _cache = ContentCacheState(
      entries: (answer['entries'] as num?)?.toInt() ?? 0,
      bytes: (answer['bytes'] as num?)?.toInt() ?? 0,
      maxBytes: (answer['max_bytes'] as num?)?.toInt() ?? 0,
    );
  }

  /// Forgets every kept document.
  Future<void> clearCache() async {
    try {
      await _readCache(clear: true);
      _problem = null;
    } on ControlFailed catch (error) {
      _problem = error.message;
    }
    notifyListeners();
  }

  /// Asks the host one question of the Try box: [method] is `content/stat`, `content/list`, `content/read` or `content/resolve`.
  /// Gives the host's answer, or throws [ControlFailed] with its words.
  Future<Map<Object?, Object?>> ask(String method, Map<String, Object?> params) async {
    final answer = _map(await _call(method, params));
    if (method == 'content/read') {
      await _readCache();
      notifyListeners();
    }
    return answer;
  }
}

String _word(Object? value) => '$value';

String _size(int bytes) {
  if (bytes < 1024) return '$bytes B';
  if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)} KiB';
  if (bytes < 1024 * 1024 * 1024) return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MiB';
  return '${(bytes / (1024 * 1024 * 1024)).toStringAsFixed(1)} GiB';
}

/// The Content page of Settings.
class ContentSection extends StatefulWidget {
  const ContentSection({super.key, required this.content});

  final ContentSettings content;

  @override
  State<ContentSection> createState() => _ContentSectionState();
}

enum _Try { stat, list, read, resolve }

class _ContentSectionState extends State<ContentSection> {
  final _uri = TextEditingController();
  _Try? _running;
  String? _failure;
  List<Widget> _result = const [];
  String? _cursor;

  @override
  void initState() {
    super.initState();
    if (!widget.content.loaded && !widget.content.loading) unawaited(widget.content.load());
  }

  @override
  void dispose() {
    _uri.dispose();
    super.dispose();
  }

  Future<void> _run(_Try what, {String? cursor, bool refresh = false}) async {
    final uri = _uri.text.trim();
    if (uri.isEmpty) return;
    setState(() {
      _running = what;
      _failure = null;
    });
    try {
      final answer = await widget.content.ask('content/${what.name}', {
        'uri': uri,
        if (what == _Try.list && cursor != null) 'cursor': cursor,
        if (what == _Try.read && refresh) 'refresh': true,
      });
      if (!mounted) return;
      setState(() {
        _running = null;
        _cursor = answer['next'] as String?;
        _result = _show(what, answer, append: cursor != null);
      });
    } on ControlFailed catch (error) {
      if (!mounted) return;
      setState(() {
        _running = null;
        _failure = error.message;
      });
    }
  }

  List<Widget> _show(_Try what, Map<Object?, Object?> answer, {required bool append}) {
    final theme = Theme.of(context);
    final mono = theme.textTheme.bodySmall?.copyWith(fontFamily: 'monospace');
    Widget line(String key, String text) => Padding(padding: const EdgeInsets.only(top: 2), child: SelectableText(text, key: Key(key), style: mono));
    Widget badge(Object? badge) => Padding(
      padding: const EdgeInsets.only(top: 4),
      child: Text(
        badge is String ? context.t('content.try.verified', {'hash': badge}) : context.t('content.try.not_verified'),
        key: const Key('content-try-badge'),
        style: theme.textTheme.labelMedium?.copyWith(color: badge is String ? theme.colorScheme.primary : theme.colorScheme.onSurfaceVariant),
      ),
    );
    switch (what) {
      case _Try.resolve:
        return [
          line('content-try-resolved', '${answer['uri']}'),
          line('content-try-revision', '${answer['revision']}${answer['immutable'] == true ? '' : ' (moves)'}'),
        ];
      case _Try.stat:
        final stat = answer['stat'] is Map ? answer['stat'] as Map<Object?, Object?> : const {};
        return [
          badge(answer['badge']),
          for (final key in ['kind', 'size', 'content_type', 'etag', 'revision', 'modified_ms'])
            if (stat[key] != null) line('content-try-$key', '$key: ${stat[key]}'),
          for (final digest in stat['digests'] as List<Object?>? ?? const [])
            if (digest is Map) line('content-try-digest-${digest['algorithm']}', '${digest['algorithm']}: ${digest['value']} (${digest['provenance']})'),
        ];
      case _Try.list:
        final entries = answer['entries'] as List<Object?>? ?? const [];
        return [
          if (append) ..._result.where((each) => each.key != const Key('content-try-empty')),
          if (entries.isEmpty && !append) Text(context.t('content.try.empty'), key: const Key('content-try-empty')),
          for (final entry in entries)
            if (entry is Map) line('content-try-entry-${entry['path']}', '${entry['kind'] == 'dir' ? '${entry['path']}/' : entry['path']}${entry['size'] == null ? '' : '  ${_size((entry['size'] as num).toInt())}'}'),
        ];
      case _Try.read:
        return [
          badge(answer['badge']),
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(
              context.t(answer['from'] == 'cache' ? 'content.try.from_cache' : 'content.try.fetched', {'size': _size((answer['bytes'] as num?)?.toInt() ?? 0)}),
              key: const Key('content-try-from'),
              style: theme.textTheme.bodySmall,
            ),
          ),
          Container(
            margin: const EdgeInsets.only(top: 6),
            padding: const EdgeInsets.all(8),
            width: double.infinity,
            decoration: BoxDecoration(border: Border.all(color: theme.colorScheme.outlineVariant), borderRadius: BorderRadius.circular(6)),
            child: SelectableText('${answer['text']}', key: const Key('content-try-text'), style: mono),
          ),
          if (answer['truncated'] == true) Text(context.t('content.try.truncated'), key: const Key('content-try-truncated'), style: theme.textTheme.bodySmall),
        ];
    }
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: widget.content,
    builder: (context, _) {
      final c = widget.content;
      final theme = Theme.of(context);
      if (!c.loaded) {
        return c.problem == null
            ? const Center(child: CircularProgressIndicator())
            : Text(context.t('content.failed', {'why': oneLine(c.problem!)}), key: const Key('content-problem'));
      }
      return Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text(context.t('content.intro'), style: theme.textTheme.bodySmall),
        if (c.problem != null) Padding(padding: const EdgeInsets.only(top: 8), child: Text(oneLine(c.problem!), key: const Key('content-problem'), style: TextStyle(color: theme.colorScheme.error))),
        const SizedBox(height: 16),
        Text(context.t('content.schemes'), style: theme.textTheme.titleSmall),
        for (final scheme in c.schemes)
          ListTile(
            key: Key('content-scheme-${scheme.scheme}'),
            contentPadding: EdgeInsets.zero,
            dense: true,
            title: Text('${scheme.scheme}:  ${scheme.grammar}'),
            subtitle: Text(
              [
                scheme.extension == null ? context.t('content.built_in') : context.t('content.brought_by', {'name': scheme.extension!}),
                scheme.provider,
                if (scheme.lists) context.t('content.lists'),
                if (scheme.resolves) context.t('content.resolves'),
                if (!scheme.on) context.t('content.off'),
              ].join(' · '),
            ),
          ),
        const SizedBox(height: 16),
        Text(context.t('content.generators'), style: theme.textTheme.titleSmall),
        if (c.generators.isEmpty) Text(context.t('content.generators_none')),
        for (final generator in c.generators)
          ListTile(
            key: Key('content-generator-${generator.id}'),
            contentPadding: EdgeInsets.zero,
            dense: true,
            title: Text(generator.title),
            subtitle: Text([generator.id, generator.extension == null ? context.t('content.built_in') : context.t('content.brought_by', {'name': generator.extension!})].join(' · ')),
          ),
        if (c.refused.isNotEmpty) ...[
          const SizedBox(height: 16),
          Text(context.t('content.refused'), style: theme.textTheme.titleSmall),
          for (final each in c.refused)
            ListTile(
              key: Key('content-refused-${each.id}'),
              contentPadding: EdgeInsets.zero,
              dense: true,
              title: Text(each.id),
              subtitle: Text(oneLine(each.why ?? '')),
            ),
        ],
        const SizedBox(height: 16),
        Text(context.t('content.cache'), style: theme.textTheme.titleSmall),
        Row(children: [
          Expanded(
            child: Text(
              context.t('content.cache.state', {'entries': '${c.cache?.entries ?? 0}', 'size': _size(c.cache?.bytes ?? 0), 'max': _size(c.cache?.maxBytes ?? 0)}),
              key: const Key('content-cache-state'),
            ),
          ),
          TextButton(key: const Key('content-cache-clear'), onPressed: (c.cache?.entries ?? 0) == 0 ? null : () => unawaited(c.clearCache()), child: Text(context.t('content.cache.clear'))),
        ]),
        const SizedBox(height: 16),
        Text(context.t('content.try'), style: theme.textTheme.titleSmall),
        const SizedBox(height: 12),
        TextField(
          key: const Key('content-try-uri'),
          controller: _uri,
          decoration: InputDecoration(labelText: context.t('content.try.uri'), helperText: context.t('content.try.uri_help')),
          onSubmitted: (_) => unawaited(_run(_Try.read)),
        ),
        const SizedBox(height: 8),
        Wrap(spacing: 8, runSpacing: 8, children: [
          for (final what in _Try.values)
            OutlinedButton(
              key: Key('content-try-${what.name}'),
              onPressed: _running == null ? () => unawaited(_run(what)) : null,
              child: Text(context.t('content.try.${what.name}')),
            ),
          OutlinedButton(
            key: const Key('content-try-refresh'),
            onPressed: _running == null ? () => unawaited(_run(_Try.read, refresh: true)) : null,
            child: Text(context.t('content.try.refresh')),
          ),
        ]),
        if (_failure != null) Padding(padding: const EdgeInsets.only(top: 8), child: Text(oneLine(_failure!), key: const Key('content-try-failure'), style: TextStyle(color: theme.colorScheme.error))),
        ..._result,
        if (_cursor != null)
          TextButton(key: const Key('content-try-more'), onPressed: _running == null ? () => unawaited(_run(_Try.list, cursor: _cursor)) : null, child: Text(context.t('content.try.more'))),
      ]);
    },
  );
}
