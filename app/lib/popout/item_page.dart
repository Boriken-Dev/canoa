import 'dart:async';

import 'package:coqui_acp_chat/experimental.dart' show ChatViewLink;
import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' as chat;
import 'package:material_ui/material_ui.dart';

import '../brand.dart';
import '../l10n/app_text.dart';
import '../surfaces/files_surface.dart';
import '../surfaces/terminal_controller.dart';
import '../surfaces/terminal_surface.dart';
import '../theme/look.dart';
import 'item_files.dart';
import 'item_terminal.dart';
import 'platform.dart' as platform;
import 'popout_launch.dart';
import 'popout_window.dart' show PopoutItem;

/// Runs this process as a window for a file or a terminal: the page that opened it holds the host connection and the shells, and
/// this window draws what that page serves over the link.
Future<void> runItemPopout(PopoutLaunch launch, {ChatViewLink? link}) async {
  WidgetsFlutterBinding.ensureInitialized();
  await platform.preparePopoutWindow(launch);
  runApp(ItemPopoutApp(launch: launch, link: link ?? await platform.connectPopout(launch)));
}

/// The app of a window for one item: Canoa's look around a file's viewer or a terminal.
class ItemPopoutApp extends StatelessWidget {
  const ItemPopoutApp({super.key, required this.launch, required this.link});

  final PopoutLaunch launch;

  /// The window's end of the link to the page that serves the item.
  final ChatViewLink link;

  @override
  Widget build(BuildContext context) {
    final look = LookId.named(launch.look) ?? LookId.canoa;
    final density = chat.ChatDensity.named(launch.density) ?? chat.ChatDensity.normal;
    final item = launch.item!;
    final language = item.args['lang'];
    return MaterialApp(
      title: '${launch.title} - ${Brand.name}',
      debugShowCheckedModeBanner: false,
      theme: themeFor(look, Brightness.light, density: density),
      darkTheme: themeFor(look, Brightness.dark, density: density),
      builder: (context, child) => AppTextHost(language: language == null || language.isEmpty ? null : language, child: child ?? const SizedBox.shrink()),
      home: switch (item.kind) {
        PopoutItem.file => _FileWindow(item: item, link: link),
        PopoutItem.terminal => _TerminalWindow(link: link),
        _ => Scaffold(body: Center(child: Builder(builder: (context) => Text(context.t('window.cannot_show', {'kind': item.kind}), key: const Key('popout-problem'))))),
      },
    );
  }
}

class _FileWindow extends StatefulWidget {
  const _FileWindow({required this.item, required this.link});

  final PopoutItem item;
  final ChatViewLink link;

  @override
  State<_FileWindow> createState() => _FileWindowState();
}

class _FileWindowState extends State<_FileWindow> {
  late final bool _writable = widget.item.args['write'] == '1';
  late final LinkedFiles _files = LinkedFiles.of(widget.link, writable: _writable);

  @override
  void dispose() {
    unawaited(_files.close());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final args = widget.item.args;
    return Scaffold(
      body: FilesPanel(
        files: _files,
        root: args['root'] ?? '',
        reveal: args['path'] == null ? null : FileReveal(args['path']!),
        canWrite: _writable,
      ),
    );
  }
}

class _TerminalWindow extends StatefulWidget {
  const _TerminalWindow({required this.link});

  final ChatViewLink link;

  @override
  State<_TerminalWindow> createState() => _TerminalWindowState();
}

class _TerminalWindowState extends State<_TerminalWindow> {
  late final LinkedTerminals _host = LinkedTerminals(widget.link, onTabs: _tabs, onTab: _tab);
  TerminalWorkspace? _workspace;
  Timer? _asking;
  var _asked = 0;
  var _gaveUp = false;

  @override
  void initState() {
    super.initState();
    // The page that holds the shells may not be listening in the first moment; it answers once it does.
    _ask();
    _asking = Timer.periodic(const Duration(milliseconds: 400), (_) => _ask());
  }

  void _ask() {
    if (_workspace != null) return;
    if (++_asked > 25) {
      _asking?.cancel();
      setState(() => _gaveUp = true);
      return;
    }
    _host.hello();
  }

  void _tabs(List<LinkedTab> tabs, int active) {
    if (_workspace != null || !mounted) return;
    _asking?.cancel();
    final workspace = TerminalWorkspace(host: _host, cwd: null);
    for (final tab in tabs) {
      _host.adopt(workspace, tab);
    }
    if (active >= 0 && active < workspace.tabs.length) workspace.activate(workspace.tabs[active]);
    setState(() => _workspace = workspace);
  }

  void _tab(LinkedTab tab) => _workspace == null ? null : _host.adopt(_workspace!, tab);

  @override
  void dispose() {
    _asking?.cancel();
    // The shells are the other page's: they are left running, and only the link is let go.
    unawaited(_host.close());
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final workspace = _workspace;
    if (workspace == null) {
      return Scaffold(
        body: Center(
          child: _gaveUp
              ? Text(context.t('window.terminal_lost'), key: const Key('popout-problem'), textAlign: TextAlign.center)
              : const CircularProgressIndicator(),
        ),
      );
    }
    return Scaffold(body: TerminalPanel(workspace: workspace, window: true, focusOnMount: true));
  }
}
