/// The kinds of tab the panel can open that the host's providers offer (the `panel_tab` kind): each says its name, icon and what a tab of
/// it shows, a shell or a log. The app knows a kind only by what its provider says of it; it draws a shell and a log itself and takes
/// everything else from the listing, so the terminal and the host's log are listed the way an extension's tab is.
library;

import 'dart:async';

import 'package:flutter/material.dart';

import '../session_store.dart';
import '../settings/host_sections.dart' show SectionCall;
import '../store/store_extensions.dart' show extensionIcons;
import 'surface.dart';

/// What a tab of a kind shows.
enum PanelShows { terminal, log }

/// One kind of tab the host lists.
final class PanelTabKind {
  const PanelTabKind({required this.id, required this.title, required this.shows, required this.origin, this.help, this.icon, this.shell});

  final String id;
  final String title;
  final PanelShows shows;

  /// `built-in`, or the name of the entry or extension that loads its provider.
  final String origin;
  final String? help;
  final String? icon;

  /// For a terminal: the id of the shell a new tab runs, one the host lists; none for the host's own choice.
  final String? shell;

  IconData get iconData => extensionIcons[icon] ?? (shows == PanelShows.log ? Icons.article_outlined : Icons.terminal);

  static PanelTabKind? read(Object? raw) {
    if (raw is! Map || raw['id'] is! String || raw['title'] is! String) return null;
    final shows = switch (raw['shows']) {
      'terminal' => PanelShows.terminal,
      'log' => PanelShows.log,
      _ => null,
    };
    if (shows == null) return null;
    String? text(Object? value) => value is String && value.isNotEmpty ? value : null;
    return PanelTabKind(
      id: raw['id'] as String,
      title: raw['title'] as String,
      shows: shows,
      origin: text(raw['origin']) ?? '',
      help: text(raw['help']),
      icon: text(raw['icon']),
      shell: text(raw['shell']),
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
