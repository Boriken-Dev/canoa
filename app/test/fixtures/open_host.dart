import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/served/host_files.dart' show HostOpener, HostOpeners, HostOpening;
import 'package:canoa/served/web_mode.dart';
import 'package:canoa/workspace/open_in.dart';
import 'package:canoa/workspace/open_in_host.dart';

/// The host this app runs, as Open in asks it: what it can start, and what it was asked to start.
final class OpenHost implements HostOpening {
  OpenHost({this.ids = const ['vscode', 'file_manager', 'reveal'], this.openers, this.os = 'linux', this.failWith});

  final List<String> ids;
  final List<HostOpener>? openers;
  final String os;
  ControlFailed? failWith;
  final started = <(String, String)>[];

  @override
  Future<HostOpeners> workspaceOpeners() async => HostOpeners(os: os, openers: openers, ids: openers == null ? ids : null);

  @override
  Future<void> workspaceOpen(String path, String opener) async {
    if (failWith case final failure?) throw failure;
    started.add((path, opener));
  }

  /// Open in as the app that runs [this] host has it, once the host has said what it can start.
  Future<WorkspaceOpener> opener() async {
    final hosted = HostedOpening(mode: WebMode.local, opening: this, hostPeer: null, page: Uri(), onThisComputer: true);
    await hosted.load();
    return WorkspaceOpener.hosted(hosted);
  }
}
