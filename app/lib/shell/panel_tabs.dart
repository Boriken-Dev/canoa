/// The kinds of tab the panel can open that the host's providers offer (the `panel_tab` kind): each says its name, icon and what a tab of
/// it shows, a shell, a log or a page. The app knows a kind only by what its provider says of it; it draws a shell and a log itself and
/// takes everything else from the listing, so the terminal and the host's log are listed the way an extension's tab is. A page is
/// opened in the person's own browser: the app has no web view of its own, and a browser tab keeps the page's sign-ins and the
/// extension's own permissions out of the app.
library;

import 'dart:async';

import 'package:flutter/material.dart';

import '../session_store.dart';
import '../settings/host_sections.dart' show SectionCall;
import '../store/links.dart' as links;
import '../store/store_extensions.dart' show extensionIcons;
import 'surface.dart';

/// What a tab of a kind shows.
enum PanelShows { terminal, log, page }

/// One kind of tab the host lists.
final class PanelTabKind {
  const PanelTabKind({required this.id, required this.title, required this.shows, required this.origin, this.help, this.icon, this.shell, this.url});

  final String id;
  final String title;
  final PanelShows shows;

  /// `built-in`, or the name of the entry or extension that loads its provider.
  final String origin;
  final String? help;
  final String? icon;

  /// For a terminal: the id of the shell a new tab runs, one the host lists; none for the host's own choice.
  final String? shell;

  /// For a page: the address the extension serves it at, on this computer's loopback.
  final Uri? url;

  IconData get iconData => extensionIcons[icon] ?? switch (shows) {
    PanelShows.log => Icons.article_outlined,
    PanelShows.page => Icons.public,
    PanelShows.terminal => Icons.terminal,
  };

  static PanelTabKind? read(Object? raw) {
    if (raw is! Map || raw['id'] is! String || raw['title'] is! String) return null;
    final shows = switch (raw['shows']) {
      'terminal' => PanelShows.terminal,
      'log' => PanelShows.log,
      'page' => PanelShows.page,
      _ => null,
    };
    if (shows == null) return null;
    String? text(Object? value) => value is String && value.isNotEmpty ? value : null;
    // A page without an address the browser may be given is no page: only a plain http or https address opens.
    final url = shows == PanelShows.page && text(raw['url']) != null ? links.webLink(text(raw['url'])!) : null;
    if (shows == PanelShows.page && url == null) return null;
    return PanelTabKind(
      id: raw['id'] as String,
      title: raw['title'] as String,
      shows: shows,
      origin: text(raw['origin']) ?? '',
      help: text(raw['help']),
      icon: text(raw['icon']),
      shell: text(raw['shell']),
      url: url,
    );
  }
}

/// The kinds the host lists, and the lines of a log kind.
final class PanelTabKinds extends ChangeNotifier {
  PanelTabKinds(this._call);

  final SectionCall _call;
  List<PanelTabKind> _kinds = const [];
  bool _disposed = false;

  /// In the order the host listed them.
  List<PanelTabKind> get kinds => _kinds;

  List<PanelTabKind> get terminals => [for (final each in _kinds) if (each.shows == PanelShows.terminal) each];
  List<PanelTabKind> get logs => [for (final each in _kinds) if (each.shows == PanelShows.log) each];
  List<PanelTabKind> get pages => [for (final each in _kinds) if (each.shows == PanelShows.page) each];

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  /// Takes the host's answer to `panel_tabs/list`.
  void adopt(Object? answer) {
    final raw = answer is Map ? answer['tabs'] : null;
    _kinds = [if (raw is List) for (final each in raw) ?PanelTabKind.read(each)];
    notifyListeners();
  }

  /// Asks the host for its kinds again; a host that cannot answer lists none.
  Future<void> load() async {
    try {
      adopt(await _call('panel_tabs/list', const {}));
    } on Object {
      adopt(null);
    }
  }

  /// The latest [lines] lines of the log kind [id], oldest first.
  Future<String> tail(String id, {int lines = 500}) async {
    final answer = await _call('panel_tabs/tail', {'id': id, 'lines': lines});
    return answer is Map && answer['text'] is String ? answer['text']! as String : '';
  }
}

/// A log the host keeps, as a surface of the panel: its latest lines, asked for again every couple of seconds while the tab is built.
class LogSurface extends WorkSurface {
  const LogSurface(this.kind, this.kinds);

  final PanelTabKind kind;
  final PanelTabKinds kinds;

  /// How often the lines are asked for again.
  static const refresh = Duration(seconds: 2);

  @override
  String get id => 'log:${kind.id}';
  @override
  String get title => kind.title;
  @override
  IconData get icon => kind.iconData;
  @override
  List<SurfaceSlot> get placements => const [SurfaceSlot.bottom, SurfaceSlot.right];

  @override
  bool available(ChatSession session) => true;

  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) => LogView(key: ValueKey('log view ${kind.id}'), kind: kind, kinds: kinds);
}

/// A page an extension serves on this computer's loopback, as a surface of the panel: its name and address and a button that opens it in
/// the person's browser. Offered only where the host is on this computer, since a loopback address is no one else's.
class PageSurface extends WorkSurface {
  const PageSurface(this.kind);

  final PanelTabKind kind;

  @override
  String get id => 'page:${kind.id}';
  @override
  String get title => kind.title;
  @override
  IconData get icon => kind.iconData;
  @override
  List<SurfaceSlot> get placements => const [SurfaceSlot.bottom, SurfaceSlot.right];

  @override
  bool available(ChatSession session) => true;

  @override
  Widget build(BuildContext context, ChatSession session, SurfaceController controller) => PageTabView(key: ValueKey('page view ${kind.id}'), kind: kind);
}

/// What a page's tab shows: where the page is and a button that opens it in the browser.
class PageTabView extends StatefulWidget {
  const PageTabView({super.key, required this.kind});

  final PanelTabKind kind;

  @override
  State<PageTabView> createState() => _PageTabViewState();
}

class _PageTabViewState extends State<PageTabView> {
  String? _problem;

  Future<void> _open() async {
    final url = widget.kind.url!;
    bool opened;
    try {
      opened = await links.openLink(url);
    } on Object {
      opened = false;
    }
    if (mounted) setState(() => _problem = opened ? null : 'No browser took $url.');
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final text = Theme.of(context).textTheme;
    final kind = widget.kind;
    return Center(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 480),
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
            Row(children: [
              Icon(kind.iconData, size: 20, color: scheme.onSurfaceVariant),
              const SizedBox(width: 8),
              Flexible(child: Text(kind.title, style: text.titleMedium)),
            ]),
            if (kind.help != null) Padding(padding: const EdgeInsets.only(top: 4), child: Text(kind.help!, style: text.bodyMedium?.copyWith(color: scheme.onSurfaceVariant))),
            Padding(padding: const EdgeInsets.only(top: 8), child: SelectableText('${kind.url}', key: const ValueKey('page address'), style: const TextStyle(fontFamily: 'Source Code Pro', fontSize: 12))),
            Padding(
              padding: const EdgeInsets.only(top: 12),
              child: FilledButton.icon(key: const ValueKey('page open'), onPressed: () => unawaited(_open()), icon: const Icon(Icons.open_in_new, size: 18), label: const Text('Open in browser')),
            ),
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text('The page opens in your browser: Canoa has no web view of its own.', style: text.bodySmall?.copyWith(color: scheme.onSurfaceVariant)),
            ),
            if (_problem case final problem?) Padding(padding: const EdgeInsets.only(top: 8), child: Text(problem, key: const ValueKey('page problem'), style: TextStyle(color: scheme.error))),
          ]),
        ),
      ),
    );
  }
}

/// The lines of a log, following the end while the person has not scrolled up.
class LogView extends StatefulWidget {
  const LogView({super.key, required this.kind, required this.kinds});

  final PanelTabKind kind;
  final PanelTabKinds kinds;

  @override
  State<LogView> createState() => _LogViewState();
}

class _LogViewState extends State<LogView> {
  String _text = '';
  String? _problem;
  bool _asked = false;
  Timer? _timer;
  final _scroll = ScrollController();

  @override
  void initState() {
    super.initState();
    unawaited(_read());
    _timer = Timer.periodic(LogSurface.refresh, (_) => unawaited(_read()));
  }

  @override
  void dispose() {
    _timer?.cancel();
    _scroll.dispose();
    super.dispose();
  }

  Future<void> _read() async {
    try {
      final text = await widget.kinds.tail(widget.kind.id);
      if (!mounted) return;
      final atEnd = !_scroll.hasClients || _scroll.position.pixels >= _scroll.position.maxScrollExtent - 24;
      setState(() {
        _text = text;
        _problem = null;
        _asked = true;
      });
      if (atEnd) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (_scroll.hasClients) _scroll.jumpTo(_scroll.position.maxScrollExtent);
        });
      }
    } on Object catch (error) {
      if (mounted) {
        setState(() {
          _problem = '$error';
          _asked = true;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    if (_problem case final problem?) {
      return Center(child: Padding(padding: const EdgeInsets.all(16), child: Text(problem, key: const ValueKey('log problem'), style: TextStyle(color: scheme.error))));
    }
    if (_asked && _text.isEmpty) {
      return Center(child: Text('Nothing has been logged yet.', key: const ValueKey('log empty'), style: TextStyle(color: scheme.onSurfaceVariant)));
    }
    return SelectionArea(
      child: Scrollbar(
        controller: _scroll,
        child: SingleChildScrollView(
          controller: _scroll,
          padding: const EdgeInsets.all(10),
          child: Align(
            alignment: Alignment.topLeft,
            child: Text(_text, key: const ValueKey('log text'), style: const TextStyle(fontFamily: 'Source Code Pro', fontSize: 12, height: 1.35)),
          ),
        ),
      ),
    );
  }
}
