import 'dart:convert';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import '../desktop/host_supervisor.dart' show HostCommandFailed;
import '../profile/aos_support.dart';
import '../profile/overlays.dart';
import '../profile/profile_home.dart';
import '../profile/project_trust.dart';

/// What profiles need of the host's computer, done by the host over its control channel: an overlay's folder read whole
/// (`profile/tree`), a folder with its links resolved (`profile/canonical`), a project's profile file (`profile/project`) and the
/// record of the projects the person trusts (`trust/*`). The app reads what the files say; the host reads the files, so a served
/// page shows the same layers, projects and trust a desktop does.
///
/// A desktop passes [local], its own home beside canoa.cfg: the host does the work while it is up, and [local] while it is not or
/// serves no control channel. The files a secret reference names are not read here: the host reads them when a session starts.
final class ControlProfileHome implements ProfileHome {
  ControlProfileHome(this._control, {required this.overlays, required this.os, this.local, this.hostUp})
      : trust = _trustOf(_control, local, os, hostUp);

  final HostControl _control;

  /// The home that reads this computer's own disk, for a desktop; none for a page.
  final ProfileHome? local;

  /// Whether the host answers now; absent means it does.
  final bool Function()? hostUp;

  @override
  final OverlayManager overlays;

  @override
  final ProjectTrust trust;

  @override
  final String os;

  @override
  String get configDir => local?.configDir ?? '';

  @override
  String join(String dir, String relative) =>
      local?.join(dir, relative) ?? (dir.isEmpty ? relative : '$dir/$relative');

  /// The host's control link, or null when [local] does the work instead. A page with no link fails the request.
  static Future<ControlClient?> _client(HostControl control, ProfileHome? local, bool Function()? hostUp) async {
    if (local != null && !(hostUp?.call() ?? true)) return null;
    try {
      return await control.ready();
    } on HostCommandFailed {
      if (local != null) return null;
      rethrow;
    }
  }

  Future<ControlClient?> _link() => _client(_control, local, hostUp);

  static Future<Map<Object?, Object?>> _call(ControlClient client, String method, Map<String, Object?> params) async {
    final result = await client.call(method, params, null, const Duration(seconds: 30));
    return result is Map<Object?, Object?> ? result : const {};
  }

  static ProjectTrust _trustOf(HostControl control, ProfileHome? local, String os, bool Function()? hostUp) {
    Future<ControlClient?> link() => _client(control, local, hostUp);
    return ProjectTrust(
      read: () async {
        final client = await link();
        if (client == null) {
          await local!.trust.load();
          return jsonEncode({
            'projects': [
              for (final project in local.trust.projects)
                {'directory': project.directory, 'sha256': project.sha256, 'trusted': project.trusted?.toUtc().toIso8601String()},
            ],
          });
        }
        return jsonEncode({'projects': (await _call(client, 'trust/list', const {}))['projects'] ?? const []});
      },
      write: (_) async => throw StateError('The host keeps the record of trusted projects'),
      caseInsensitive: os == 'windows' || os == 'macos',
      trustOn: (directory, profile) async {
        final client = await link();
        if (client == null) return local!.trust.trust(directory, profile);
        try {
          await _call(client, 'trust/set', {'directory': directory, 'sha256': ProjectTrust.digest(profile)});
        } on ControlFailed catch (error) {
          throw StateError(error.message);
        }
      },
      revokeOn: (directory) async {
        final client = await link();
        if (client == null) return local!.trust.revoke(directory);
        try {
          await _call(client, 'trust/revoke', {'directory': directory});
        } on ControlFailed catch (error) {
          throw StateError(error.message);
        }
      },
    );
  }

  @override
  Future<String?> canonicalDirectory(String directory) async {
    final client = await _link();
    if (client == null) return local!.canonicalDirectory(directory);
    try {
      return (await _call(client, 'profile/canonical', {'path': directory}))['directory'] as String?;
    } on ControlFailed {
      return null;
    }
  }

  /// A project's profile file: the one thing of the files this page reads, and only through `profile/project`.
  @override
  Future<List<int>?> readBytes(String path) async {
    final client = await _link();
    if (client == null) return local!.readBytes(path);
    const file = '/$projectProfileFile';
    final normal = path.replaceAll('\\', '/');
    if (!normal.endsWith(file)) return null;
    try {
      final answer = await _call(client, 'profile/project', {'directory': path.substring(0, path.length - file.length)});
      return answer['profile'] is String ? base64Decode(answer['profile']! as String) : null;
    } on ControlFailed {
      return null;
    }
  }

  @override
  Future<ReadTree> readTree(String directory, {int maxFiles = 4096, int maxBytes = 64 * 1024 * 1024}) async {
    final client = await _link();
    if (client == null) return local!.readTree(directory, maxFiles: maxFiles, maxBytes: maxBytes);
    final Map<Object?, Object?> answer;
    try {
      answer = await _call(client, 'profile/tree', {'directory': directory});
    } on ControlFailed catch (error) {
      throw aos.AosError('io', error.message);
    }
    final files = <String, List<int>>{
      for (final MapEntry(:key, :value) in (answer['files'] as Map<Object?, Object?>? ?? const {}).entries)
        '$key': base64Decode('$value'),
    };
    if (files.length > maxFiles || files.values.fold<int>(0, (all, bytes) => all + bytes.length) > maxBytes) {
      throw aos.AosError('too-large', 'The overlay holds more than $maxFiles files or $maxBytes bytes');
    }
    return treeOfFiles(files);
  }

  @override
  Future<String?> readSecret(String path, {String? within}) async {
    if (local != null && (await _link()) == null) return local!.readSecret(path, within: within);
    return null;
  }
}

