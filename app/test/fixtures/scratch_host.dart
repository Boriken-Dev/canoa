// A stand-in for the host's chat folders (`fs/scratch*`): it expands the workspace template as the host does and makes the folder
// on this disk, so a test of what the app does around it needs no host process. The host's own work is tested in canoa-host.
import 'dart:io';

import 'package:canoa/desktop/host_control.dart';
import 'package:canoa/sources/sources.dart';
import 'package:canoa/workspace/documents.dart';
import 'package:canoa/workspace/host_folders.dart';
import 'package:canoa/workspace/scratch.dart';
import 'package:canoa/workspace/site.dart';
import 'package:canoa/workspace/workspace_template.dart';

/// The folder [template] makes for the session [sessionId] started from [source], or null where this build has no folders
/// or no Documents folder to put it in. The arguments stand in for the system's, for a test.
String? scratchFolder(String template, String sessionId, {
  AgentSource? source,
  DateTime? now,
  String? documents,
  String? home,
  String? host,
  String? separator,
  String? fqdn,
}) {
  // A site name is taken only as [siteName] writes it: one folder name, so it cannot reach outside the scratch root.
  if (fqdn != null && siteName(fqdn) != fqdn) {
    throw ArgumentError.value(fqdn, 'fqdn', 'is not a host name that is safe as a folder name');
  }
  // A page has no folders of its own; the folder it is given is on the machine its agent runs on.
  if (!hasWorkspaceFolders && documents == null) return null;
  final root = documents ?? documentsFolder();
  if (root == null && template.contains(r'${documents}')) return null;
  return expandWorkspace(
    template,
    WorkspaceValues(
      documents: root ?? '',
      home: home ?? homeFolder() ?? '',
      session: sessionId,
      now: (now ?? DateTime.now()).toLocal(),
      handler: source == null ? '' : handlerOf(source),
      source: source?.label ?? '',
      host: host ?? localHostName() ?? '',
      fqdn: fqdn ?? '',
    ),
    separator: separator ?? (hasWorkspaceFolders ? pathSeparator : separatorOf(root ?? '')),
  );
}


/// Chat folders made under [documents], for a template that names `${documents}`.
final class ScratchHost extends HostFolders {
  ScratchHost(this.documents, {this.template = defaultWorkspace}) : super(HostControl.dialing(() async => throw StateError('a stand-in host has no control channel')));

  final String documents;

  /// What `[sessions] workspace` holds, when the app is not given another.
  final String template;

  /// Every folder it was asked to make.
  final made = <String>[];

  String _folder(String template, ScratchRequest request) => scratchFolder(
        template,
        request.session,
        documents: documents,
        now: request.created,
        source: request.source.isEmpty ? null : AgentSource.hosted(request.source, harness: request.handler),
      )!;

  @override
  Future<({String path, bool made})> makeScratch(ScratchRequest request) async {
    final path = _folder(template, request);
    final fresh = !Directory(path).existsSync();
    Directory(path).createSync(recursive: true);
    made.add(path);
    return (path: path, made: fresh);
  }

  @override
  Future<void> removeScratch(ScratchRequest request) async {
    final directory = Directory(_folder(template, request));
    if (directory.existsSync() && directory.listSync().isEmpty) directory.deleteSync();
  }

  @override
  Future<String?> preview(String template, {String source = '', String handler = ''}) async => scratchFolder(
        template,
        exampleSessionId,
        documents: documents,
        source: source.isEmpty ? null : AgentSource.hosted(source, harness: handler),
      );
}
