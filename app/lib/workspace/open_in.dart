import 'package:flutter/material.dart';

import '../acp/pipe_channel.dart' show isPipeUrl;
import '../ledger/session_ledger.dart' show SessionPlace;
import '../served/web_mode.dart' show WebMode;
import '../store/links.dart' as links;
import 'open_in_host.dart';
import 'open_in_stub.dart' if (dart.library.io) 'open_in_io.dart' as platform;

/// Where a workspace action is offered.
enum WorkspaceContext { header, palette, chip, files, browser }

enum HostOs { windows, macos, linux }

/// What a workspace action works on: a folder of this computer, or, with [item], something inside one that a file manager is
/// asked to select.
final class WorkspaceTarget {
  const WorkspaceTarget(this.path, {this.item = false});
  final String path;
  final bool item;
}

/// A program and its arguments, started as they are: never through a shell, never joined into a command line.
final class LaunchPlan {
  const LaunchPlan(this.executable, this.arguments);
  final String executable;
  final List<String> arguments;

  @override
  bool operator ==(Object other) =>
      other is LaunchPlan && other.executable == executable && _same(other.arguments, arguments);

  @override
  int get hashCode => Object.hash(executable, Object.hashAll(arguments));

  @override
  String toString() => '$executable $arguments';

  static bool _same(List<String> a, List<String> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }
}

/// An action that could not be done; [message] is for the person.
final class OpenInFailure implements Exception {
  OpenInFailure(this.message);
  final String message;

  @override
  String toString() => message;
}

/// What the actions need from the machine: its facts, and starting a program. Tests replace it.
abstract interface class OpenInSystem {
  /// This computer's system; null where a page runs, which cannot start programs.
  HostOs? get os;

  Map<String, String> get environment;
  bool isDirectory(String path);
  bool isFile(String path);

  /// The default value of a Windows registry key under `HKEY_...`, else null.
  String? registryDefault(String key);

  /// The folder of the application bundle named [id] on a Mac, else null.
  String? bundlePath(String id);

  /// Starts [executable] with [arguments] and does not wait for it; throws [OpenInFailure] when it cannot.
  Future<void> start(String executable, List<String> arguments);
}

/// What a page has for the system: nothing, since it starts no program itself.
final class _NoSystem implements OpenInSystem {
  const _NoSystem();

  @override
  HostOs? get os => null;
  @override
  Map<String, String> get environment => const {};
  @override
  bool isDirectory(String path) => false;
  @override
  bool isFile(String path) => false;
  @override
  String? registryDefault(String key) => null;
  @override
  String? bundlePath(String id) => null;
  @override
  Future<void> start(String executable, List<String> arguments) =>
      Future.error(OpenInFailure('A page cannot open programs on the computer it is served from.'));
}

/// The system this program runs on.
OpenInSystem systemOpenIn() => platform.systemOpenIn();

/// One thing the person can do with a workspace.
final class WorkspaceAction {
  const WorkspaceAction({
    required this.id,
    required this.label,
    required this.icon,
    required this.contexts,
    required this.available,
    required this.run,
  });

  final String id;
  final String label;
  final IconData icon;
  final Set<WorkspaceContext> contexts;

  /// Whether it can be done for [target] now.
  final bool Function(WorkspaceTarget target) available;

  /// Does it; throws [OpenInFailure]. It checks the target again: a caller's check is not trusted.
  final Future<void> Function(WorkspaceTarget target) run;
}

const _vsCodeBundle = 'com.microsoft.VSCode';
const _appPaths = r'\Software\Microsoft\Windows\CurrentVersion\App Paths\Code.exe';

/// The path as the system's file manager and editor take it, or null when it cannot be a file or folder they open: relative, with a
/// `..` part, or holding a control character. Whatever the system's own file manager opens is opened, a network share included.
String? localPathOf(String path, HostOs os) {
  if (path.isEmpty || path.length > 4096) return null;
  if (path.codeUnits.any((unit) => unit < 0x20 || unit == 0x7f)) return null;
  if (os == HostOs.windows) {
    // A drive path, or one that starts with two separators: a share, or a verbatim or device path.
    if (!RegExp(r'^([A-Za-z]:[\\/]|[\\/][\\/][^\\/])').hasMatch(path)) return null;
    final windows = path.replaceAll('/', r'\');
    if (windows.split(r'\').contains('..')) return null;
    return windows.length > 3 && windows.endsWith(r'\') ? windows.substring(0, windows.length - 1) : windows;
  }
  if (!path.startsWith('/')) return null;
  if (path.split('/').contains('..')) return null;
  return path.length > 1 && path.endsWith('/') ? path.substring(0, path.length - 1) : path;
}

/// The value a `reg query ... /ve` printed, with `%NAME%` filled in from [environment]; null when it printed none.
String? parseRegistryDefault(String output, Map<String, String> environment) {
  final match = RegExp(r'REG_(EXPAND_)?SZ[ \t]+(.+?)[ \t]*$', multiLine: true).firstMatch(output);
  if (match == null) return null;
  var value = match.group(2)!;
  if (value.length > 1 && value.startsWith('"') && value.endsWith('"')) value = value.substring(1, value.length - 1);
  if (match.group(1) != null) {
    value = value.replaceAllMapped(RegExp('%([^%]+)%'), (found) {
      final name = found.group(1)!.toUpperCase();
      for (final entry in environment.entries) {
        if (entry.key.toUpperCase() == name) return entry.value;
      }
      return found.group(0)!;
    });
  }
  return value.isEmpty ? null : value;
}

/// The workspace actions, detected on [system].
final class WorkspaceOpener {
  WorkspaceOpener(this.system, {Future<bool> Function(Uri uri)? launch})
      : hosted = null,
        _launch = launch;

  /// A page a host serves: the host starts the program, or the page links to it, by the host's mode.
  WorkspaceOpener.hosted(HostedOpening this.hosted, {Future<bool> Function(Uri uri)? launch})
      : system = const _NoSystem(),
        _launch = launch;

  /// Hands an address to the system's browser; [links.openLink] unless a test gave another.
  final Future<bool> Function(Uri uri)? _launch;

  final OpenInSystem system;

  /// How a served page opens a workspace; null for the app that runs on the computer itself.
  final HostedOpening? hosted;

  /// Whether this build can start programs at all. A page nothing served cannot, and shows no action.
  bool get canOpen => hosted != null || system.os != null;

  /// What to tell the person where a workspace action they might look for is not offered.
  String? get note => hosted?.mode == WebMode.remote ? HostedOpening.remoteNote : null;

  String? _env(String name) {
    final os = system.os;
    if (os == HostOs.windows) {
      final upper = name.toUpperCase();
      for (final entry in system.environment.entries) {
        if (entry.key.toUpperCase() == upper && entry.value.isNotEmpty) return entry.value;
      }
      return null;
    }
    final value = system.environment[name];
    return value == null || value.isEmpty ? null : value;
  }

  String _join(String folder, String name) {
    final separator = system.os == HostOs.windows ? r'\' : '/';
    return folder.endsWith(separator) ? '$folder$name' : '$folder$separator$name';
  }

  /// A file called [name] in a folder of PATH.
  String? onPath(String name) {
    final path = _env('PATH');
    if (path == null) return null;
    for (final folder in path.split(system.os == HostOs.windows ? ';' : ':')) {
      if (folder.isEmpty) continue;
      final candidate = _join(folder.replaceAll('"', ''), name);
      if (system.isFile(candidate)) return candidate;
    }
    return null;
  }

  /// The program that opens a folder in VS Code, as a path: `Code.exe` on Windows (never `code.cmd`, which only a shell can
  /// run), `open` with the bundle's path on a Mac, `code` on Linux. Null when VS Code is not found.
  LaunchPlan? vsCodePlan(String path) {
    switch (system.os) {
      case HostOs.windows:
        final candidates = <String?>[
          for (final root in const ['HKCU', 'HKLM']) system.registryDefault('$root$_appPaths'),
          if (_env('LOCALAPPDATA') case final local?) _join(local, r'Programs\Microsoft VS Code\Code.exe'),
          if (_env('ProgramFiles') case final files?) _join(files, r'Microsoft VS Code\Code.exe'),
          if (onPath('code.cmd') case final cmd?) _join(cmd.substring(0, cmd.lastIndexOf(RegExp(r'[\\/]'))), r'..\Code.exe'),
          onPath('Code.exe'),
        ];
        for (final candidate in candidates) {
          if (candidate != null && candidate.toLowerCase().endsWith('code.exe') && system.isFile(candidate)) {
            return LaunchPlan(candidate, [path]);
          }
        }
        return null;
      case HostOs.macos:
        final bundle = system.bundlePath(_vsCodeBundle);
        return bundle == null ? null : LaunchPlan('open', ['-a', bundle, path]);
      case HostOs.linux:
        final code = onPath('code');
        return code == null ? null : LaunchPlan(code, [path]);
      case null:
        return null;
    }
  }

  /// What the file manager runs to show [path]: its folder opened, or with [item] the entry selected in its parent.
  LaunchPlan? fileManagerPlan(String path, {bool item = false}) {
    switch (system.os) {
      case HostOs.windows:
        final root = _env('SystemRoot');
        final explorer = root == null ? 'explorer.exe' : _join(root, 'explorer.exe');
        // `/select,` and the path stay two arguments: the path is one argument whatever it holds.
        return LaunchPlan(explorer, item ? ['/select,', path] : [path]);
      case HostOs.macos:
        return LaunchPlan('open', item ? ['-R', path] : [path]);
      case HostOs.linux:
        if (!item) return LaunchPlan('xdg-open', [path]);
        final gdbus = onPath('gdbus');
        if (gdbus != null) {
          // GVariant text names the item as a URI, whose `'` is escaped since the text would end there.
          final uri = Uri.file(path).toString().replaceAll("'", '%27');
          return LaunchPlan(gdbus, [
            'call', '--session', '--dest', 'org.freedesktop.FileManager1', '--object-path', '/org/freedesktop/FileManager1',
            '--method', 'org.freedesktop.FileManager1.ShowItems', "['$uri']", '',
          ]);
        }
        final slash = path.lastIndexOf('/');
        return LaunchPlan('xdg-open', [slash <= 0 ? '/' : path.substring(0, slash)]);
      case null:
        return null;
    }
  }

  String get _managerName => switch (system.os) {
    HostOs.windows => 'Explorer',
    HostOs.macos => 'Finder',
    _ => 'file manager',
  };

  /// [target]'s path as this system takes it, when it is a folder this system finds (or, for an item, a file or folder); throws
  /// [OpenInFailure] otherwise.
  String checked(WorkspaceTarget target) {
    final os = system.os;
    if (os == null) throw OpenInFailure('This page cannot open programs on the computer it is served from.');
    final path = localPathOf(target.path, os);
    if (path == null) {
      throw OpenInFailure('That is not a path this computer can open (it is absolute, with no ".." and no control character): ${target.path}');
    }
    final exists = target.item ? system.isDirectory(path) || system.isFile(path) : system.isDirectory(path);
    if (!exists) throw OpenInFailure(target.item ? 'It is not there any more: $path' : 'The folder is not there: $path');
    return path;
  }

  Future<void> _run(LaunchPlan plan) async {
    final name = plan.executable.toLowerCase();
    if (name.endsWith('.cmd') || name.endsWith('.bat') || name.endsWith('.com')) {
      throw OpenInFailure('${plan.executable} can only be started through a shell, which Canoa does not use.');
    }
    await system.start(plan.executable, plan.arguments);
  }

  List<WorkspaceAction> get actions => [...(hosted?.actions ?? _local), _browser];

  /// The address [text] names as the person typed it: a bare host such as `localhost:3000` or `example.com/a` is taken as https. Null
  /// unless it is a plain `http` or `https` address with a host, so a `file:`, `javascript:` or `data:` address is never opened.
  static Uri? browserUrl(String text) {
    final trimmed = text.trim();
    final bare = RegExp(r'^[A-Za-z0-9][A-Za-z0-9.-]*(:\d+)?([/?#]|$)').hasMatch(trimmed) && !trimmed.contains('://');
    return links.webLink(bare ? 'https://$trimmed' : trimmed);
  }

  /// Opens [text] in the system's browser, where the extension's side panel shares its tabs under its own permissions; throws
  /// [OpenInFailure] for an address that is not a web one or that nothing took.
  Future<void> openUrl(String text) async {
    final uri = browserUrl(text);
    if (uri == null) throw OpenInFailure('Only an http or https address opens in the browser: $text');
    final opened = await (_launch ?? links.openLink)(uri);
    if (!opened) throw OpenInFailure('The system did not open $uri.');
  }

  /// Takes an address as a target's path, in its own context: the person gives it, or a tool card links it.
  late final WorkspaceAction _browser = WorkspaceAction(
    id: 'browser',
    label: 'Open in browser',
    icon: Icons.open_in_browser,
    contexts: const {WorkspaceContext.browser},
    available: (target) => browserUrl(target.path) != null,
    run: (target) => openUrl(target.path),
  );

  late final List<WorkspaceAction> _local = [
    WorkspaceAction(
      id: 'vscode',
      label: 'Open in VS Code',
      icon: Icons.code,
      contexts: const {WorkspaceContext.header, WorkspaceContext.palette, WorkspaceContext.chip, WorkspaceContext.files},
      available: (target) {
        final os = system.os;
        final path = os == null ? null : localPathOf(target.path, os);
        return path != null && vsCodePlan(path) != null;
      },
      run: (target) async {
        final path = checked(target);
        final plan = vsCodePlan(path);
        if (plan == null) throw OpenInFailure('VS Code was not found on this computer.');
        await _run(plan);
      },
    ),
    WorkspaceAction(
      id: 'file_manager',
      label: 'Open in $_managerName',
      icon: Icons.folder_open,
      contexts: const {WorkspaceContext.header, WorkspaceContext.palette, WorkspaceContext.chip},
      available: (target) => canOpen && !target.item,
      run: (target) async => _run(fileManagerPlan(checked(target))!),
    ),
    // For a place that holds a file or folder of the workspace, such as the Files surface.
    WorkspaceAction(
      id: 'reveal',
      label: 'Reveal in $_managerName',
      icon: Icons.folder_open,
      contexts: const {WorkspaceContext.files},
      available: (target) => canOpen && target.item,
      run: (target) async => _run(fileManagerPlan(checked(target), item: true)!),
    ),
  ];

  /// What is offered for [target] in [context], in order.
  List<WorkspaceAction> actionsFor(WorkspaceContext context, WorkspaceTarget target) =>
      [for (final action in actions) if (action.contexts.contains(context) && action.available(target)) action];

  /// A session's folder as a target, or why it is not one: a session on another host has its folder there, not here.
  /// Both are null where nothing can be started, or when no folder is known.
  ({WorkspaceTarget? target, String? reason}) workspaceOf({required SessionPlace place, required String? cwd, Uri? endpoint, String? peerId}) {
    if (!canOpen || cwd == null || cwd.isEmpty) return (target: null, reason: null);
    if (hosted case final host?) {
      if (place == SessionPlace.browser) {
        return (target: null, reason: 'This conversation runs through the browser, so its folder is not one of this host\'s.');
      }
      if (peerId != null && peerId != host.hostPeer) {
        return (target: null, reason: 'This workspace is on another host, so it cannot be opened from this one.');
      }
      return (target: WorkspaceTarget(cwd), reason: null);
    }
    final here = place == SessionPlace.local || ((place == SessionPlace.host || place == SessionPlace.peer) && endpoint != null && isPipeUrl(endpoint.toString()));
    if (!here) {
      return (
        target: null,
        reason: place == SessionPlace.browser
            ? 'This conversation runs through the browser, so its folder is not one of this computer\'s.'
            : 'This workspace is on another host, so it cannot be opened on this computer.',
      );
    }
    return (target: WorkspaceTarget(cwd), reason: null);
  }
}

/// The actions the app offers; tests replace it.
WorkspaceOpener workspaceOpener = WorkspaceOpener(systemOpenIn());
