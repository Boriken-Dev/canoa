import 'package:flutter/material.dart';

import '../acp/pipe_channel.dart' show isPipeUrl;
import '../desktop/control_client.dart' show ControlFailed;
import '../ledger/session_ledger.dart' show SessionPlace;
import '../served/host_files.dart' show HostOpening, HostOpeners;
import '../served/web_mode.dart';
import '../store/links.dart' as links;
import 'open_in.dart';

/// Open in for an app that reaches a host, by the mode that host says the page is in; the app that runs the host is in the local mode. Where the person sits at the host's computer
/// ([WebMode.local]) the host starts VS Code or the file manager there. Where the browser is elsewhere ([WebMode.remote]) VS Code
/// is a Remote-SSH link the browser opens, and the host's file manager is not reachable, so the Files panel stays the way in.
final class HostedOpening {
  HostedOpening({
    required this.mode,
    required this.opening,
    required this.hostPeer,
    required this.page,
    this.onThisComputer = false,
    Future<bool> Function(Uri)? link,
  }) : _link = link;

  final WebMode mode;
  final HostOpening opening;

  /// The host's peer id: a session on another peer's host is not this host's folder.
  final String? hostPeer;

  /// Whether the app runs this host itself, so a session of this computer (local, or reached through the host's own pipe) is the
  /// host's, as a session on a page is by its peer id.
  final bool onThisComputer;

  /// The address this page was reached at; its host name is what Remote-SSH connects to.
  final Uri page;
  final Future<bool> Function(Uri)? _link;

  HostOpeners? _openers;

  /// What a remote page tells the person about the file manager.
  static const remoteNote = 'This host\'s file manager is on another computer: browse the folder in the Files panel.';

  /// Asks the host what it can start; only a local page needs it. A host that refuses (no grant) or cannot say leaves nothing offered.
  Future<void> load() async {
    if (mode != WebMode.local) return;
    try {
      _openers = await opening.workspaceOpeners();
    } on Object {
      _openers = null;
    }
  }

  /// [cwd] as a target when the session is on this host, else why it is not one.
  ({WorkspaceTarget? target, String? reason}) folderOf({required SessionPlace place, required String cwd, Uri? endpoint, String? peerId}) {
    if (onThisComputer) {
      final here = place == SessionPlace.local ||
          ((place == SessionPlace.host || place == SessionPlace.peer) && endpoint != null && isPipeUrl(endpoint.toString()));
      if (here) return (target: WorkspaceTarget(cwd), reason: null);
      return (
        target: null,
        reason: place == SessionPlace.browser
            ? 'This conversation runs through the browser, so its folder is not one of this computer\'s.'
            : 'This workspace is on another host, so it cannot be opened on this computer.',
      );
    }
    if (place == SessionPlace.browser) {
      return (target: null, reason: 'This conversation runs through the browser, so its folder is not one of this host\'s.');
    }
    if (peerId != null && peerId != hostPeer) {
      return (target: null, reason: 'This workspace is on another host, so it cannot be opened from this one.');
    }
    return (target: WorkspaceTarget(cwd), reason: null);
  }

  bool _has(String id) => _openers?.ids.contains(id) ?? false;

  String get _managerName => switch (_openers?.os) {
    'windows' => 'Explorer',
    'macos' => 'Finder',
    _ => 'file manager',
  };

  /// `vscode://vscode-remote/ssh-remote+HOST/PATH`: the folder on the host as VS Code's Remote-SSH opens it. A Windows path
  /// `C:\a\b` is `/C:/a/b`.
  static Uri vsCodeRemote(String host, String path) {
    final slashed = path.replaceAll('\\', '/');
    final rooted = slashed.startsWith('/') ? slashed : '/$slashed';
    final encoded = rooted.split('/').map((part) => part.contains(':') && part.length == 2 ? part : Uri.encodeComponent(part)).join('/');
    return Uri.parse('vscode://vscode-remote/ssh-remote+${Uri.encodeComponent(host)}$encoded');
  }

  static bool _absolute(String path) => path.startsWith('/') || RegExp(r'^[A-Za-z]:[\\/]').hasMatch(path) || path.startsWith(r'\\');

  List<WorkspaceAction> get actions => [
    WorkspaceAction(
      id: 'vscode',
      label: 'Open in VS Code',
      icon: Icons.code,
      contexts: const {WorkspaceContext.header, WorkspaceContext.palette, WorkspaceContext.chip, WorkspaceContext.files},
      available: (target) => _absolute(target.path) && (mode == WebMode.remote ? page.host.isNotEmpty : _has('vscode')),
      run: (target) async {
        if (mode == WebMode.remote) {
          if (page.host.isEmpty) throw OpenInFailure('This page has no host name to connect VS Code to.');
          final opened = await (_link ?? links.openLink)(vsCodeRemote(page.host, target.path));
          if (!opened) throw OpenInFailure('The browser did not open VS Code. Install the Remote - SSH extension and allow the link.');
          return;
        }
        await _start(target.path, 'vscode');
      },
    ),
    WorkspaceAction(
      id: 'file_manager',
      label: 'Open in $_managerName',
      icon: Icons.folder_open,
      contexts: const {WorkspaceContext.header, WorkspaceContext.palette, WorkspaceContext.chip},
      available: (target) => mode == WebMode.local && _has('file_manager') && !target.item,
      run: (target) => _start(target.path, 'file_manager'),
    ),
    WorkspaceAction(
      id: 'reveal',
      label: 'Reveal in $_managerName',
      icon: Icons.folder_open,
      contexts: const {WorkspaceContext.files},
      available: (target) => mode == WebMode.local && _has('reveal') && target.item,
      run: (target) => _start(target.path, 'reveal'),
    ),
  ];

  Future<void> _start(String path, String opener) async {
    try {
      await opening.workspaceOpen(path, opener);
    } on ControlFailed catch (failure) {
      throw OpenInFailure(failure.message);
    } on Object catch (error) {
      throw OpenInFailure('The host could not be asked: $error');
    }
  }
}
