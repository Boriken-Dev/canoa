import 'package:flutter/material.dart';

import '../ledger/session_ledger.dart' show SessionPlace;
import '../served/web_mode.dart' show WebMode;
import '../store/links.dart' as links;
import 'open_in_host.dart';

/// Where a workspace action is offered.
enum WorkspaceContext { header, palette, chip, files, browser }

/// What a workspace action works on: a folder of this computer, or, with [item], something inside one that a file manager is
/// asked to select.
final class WorkspaceTarget {
  const WorkspaceTarget(this.path, {this.item = false});
  final String path;
  final bool item;
}

/// An action that could not be done; [message] is for the person.
final class OpenInFailure implements Exception {
  OpenInFailure(this.message);
  final String message;

  @override
  String toString() => message;
}

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

/// The workspace actions: the host this app reaches starts the program (or the page links to it), by its mode.
final class WorkspaceOpener {
  /// An app that reaches no host: it offers only what needs none.
  const WorkspaceOpener.none({Future<bool> Function(Uri uri)? launch})
      : hosted = null,
        _launch = launch;

  WorkspaceOpener.hosted(HostedOpening this.hosted, {Future<bool> Function(Uri uri)? launch}) : _launch = launch;

  /// Hands an address to the system's browser; [links.openLink] unless a test gave another.
  final Future<bool> Function(Uri uri)? _launch;

  /// How the host opens a workspace; null where no host is reached.
  final HostedOpening? hosted;

  /// Whether this app can have a program started at all.
  bool get canOpen => hosted != null;

  /// What to tell the person where a workspace action they might look for is not offered.
  String? get note => hosted?.mode == WebMode.remote ? HostedOpening.remoteNote : null;

  /// Open in browser is offered on every build: it opens an address in the person's own browser and needs no host.
  List<WorkspaceAction> get actions => [...(hosted?.actions ?? const <WorkspaceAction>[]), _browser];

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
  WorkspaceAction get _browser => WorkspaceAction(
    id: 'browser',
    label: 'Open in browser',
    icon: Icons.open_in_browser,
    contexts: const {WorkspaceContext.browser},
    available: (target) => browserUrl(target.path) != null,
    run: (target) => openUrl(target.path),
  );

  /// What is offered for [target] in [context], in order.
  List<WorkspaceAction> actionsFor(WorkspaceContext context, WorkspaceTarget target) =>
      [for (final action in actions) if (action.contexts.contains(context) && action.available(target)) action];

  /// A session's folder as a target, or why it is not one: a session on another host has its folder there, not here.
  /// Both are null where nothing can be started, or when no folder is known.
  ({WorkspaceTarget? target, String? reason}) workspaceOf({required SessionPlace place, required String? cwd, Uri? endpoint, String? peerId}) {
    final host = hosted;
    if (host == null || cwd == null || cwd.isEmpty) return (target: null, reason: null);
    return host.folderOf(place: place, cwd: cwd, endpoint: endpoint, peerId: peerId);
  }
}

/// The actions the app offers; tests replace it.
WorkspaceOpener workspaceOpener = const WorkspaceOpener.none();
