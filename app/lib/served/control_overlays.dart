import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import '../profile/overlays.dart';
import '../profile/profile.dart';
import '../profile/skill_copies.dart';

/// The overlay store's git work done by the host over its control channel (`overlay/*` and `catalog/*`), for a page that cannot run
/// git: the host fetches a source into its own cache, reads what it holds from git's objects without checking anything out,
/// digests it as AOS defines and installs it beside its canoa.cfg. What the store shows and decides stays this app's, over the
/// same [OverlayManager] the desktop's own store implements.
///
/// [installed] answers from what [refresh] last read, since a page cannot read the host's disk in a call that is not a request.
/// Skills fetched from elsewhere are not part of it: the host fetches none for a page.
final class ControlOverlayManager implements OverlayManager {
  ControlOverlayManager(this._control, {this.root = 'overlays'});

  final HostControl _control;

  /// Where the host keeps overlays, as it said; only for showing.
  final String root;

  final _installed = <String, InstalledOverlay>{};

  /// The host's plan for each check, by the update that showed it.
  final _plans = Expando<String>('overlay plan');

  Future<Map<Object?, Object?>> _call(String method, Map<String, Object?> params, [Duration? wait]) async {
    final client = await _control.ready();
    try {
      final result = await client.call(method, params, null, wait ?? const Duration(minutes: 6));
      return result is Map<Object?, Object?> ? result : const {};
    } on ControlFailed catch (error) {
      throw OverlayException(error.message);
    }
  }

  /// Reads what the host has installed.
  Future<void> refresh() async {
    final answer = await _call('overlay/installed', const {});
    final found = <String, InstalledOverlay>{};
    for (final MapEntry(:key, :value) in (answer['overlays'] as Map<Object?, Object?>? ?? const {}).entries) {
      if (key is! String || value is! Map) continue;
      try {
        found[key] = InstalledOverlay.fromJson(key, value.cast<String, dynamic>());
      } on Object {
        // A record this app cannot read is no overlay it can offer.
      }
    }
    _installed
      ..clear()
      ..addAll(found);
  }

  @override
  InstalledOverlay? installed(String name) => _installed[name];

  @override
  String directoryOf(String name) => '$root/$name';

  @override
  Future<OverlayUpdate> check(String name, GitOverlay given) async {
    final answer = await _call('overlay/check', {
      'name': name,
      ..._sourceOf(given),
      'pin': ?given.pin,
    });
    final files = {for (final MapEntry(:key, :value) in (answer['files'] as Map<Object?, Object?>).entries) '$key': '$value'};
    List<String> names(String key) => [for (final each in (answer[key] as List<Object?>? ?? const [])) '$each'];
    final update = OverlayUpdate(
      name: name,
      source: given,
      from: answer['from'] as String?,
      fromSource: answer['fromSource'] as String?,
      fromDigest: answer['fromDigest'] as String?,
      commit: answer['commit']! as String,
      digest: answer['digest']! as String,
      files: files,
      skipped: names('skipped'),
      added: names('added'),
      changed: names('changed'),
      removed: names('removed'),
      manifest: answer['manifest'] as String?,
    );
    _plans[update] = answer['token']! as String;
    return update;
  }

  String _planOf(OverlayUpdate update) =>
      _plans[update] ?? (throw const OverlayException('That overlay was not checked on this host: check it again'));

  @override
  Future<InstalledOverlay> apply(OverlayUpdate update, {String? catalog}) async {
    final answer = await _call('overlay/apply', {'token': _planOf(update), 'catalog': ?catalog});
    final record = InstalledOverlay.fromJson(update.name, answer.cast<String, dynamic>());
    _installed[update.name] = record;
    return record;
  }

  @override
  Future<void> remove(String name) async {
    await _call('overlay/remove', {'name': name});
    _installed.remove(name);
  }

  @override
  Future<Map<String, List<int>>> read(OverlayUpdate update) async {
    final answer = await _call('overlay/read', {'token': _planOf(update)});
    return {
      for (final MapEntry(:key, :value) in (answer['files'] as Map<Object?, Object?>).entries) '$key': base64Decode('$value'),
    };
  }

  @override
  Future<void> discard(String name) async {
    await _call('overlay/discard', {'name': name});
  }

  @override
  Future<FetchedTree> fetchCatalog(String label, GitOverlay source) async {
    final answer = await _call('catalog/fetch', {'label': label, ..._sourceOf(source)});
    return _RemoteTree(this, label, answer['commit']! as String);
  }

  @override
  Future<void> removeCatalog(String label) async {
    await _call('catalog/remove', {'label': label});
  }

  @override
  Future<Map<String, SkillCopy>> skillCopies() async => const {};

  @override
  Future<List<(String, Uint8List)>> fetchSkillFiles(aos.Source source) =>
      throw const OverlayException('This host fetches no skills for a web page');

  @override
  Future<void> saveSkill(SkillCopy copy) => throw const OverlayException('This host keeps no skills for a web page');

  @override
  Future<void> removeSkill(String name) => throw const OverlayException('This host keeps no skills for a web page');

  static Map<String, Object?> _sourceOf(GitOverlay source) => {
    'repo': source.repo,
    'ref': source.reference,
    'path': source.path,
    'spec': source.spec,
  };
}

/// A catalog repository at one commit the host fetched, read through the host.
final class _RemoteTree implements FetchedTree {
  _RemoteTree(this._manager, this._label, this.commit);

  final ControlOverlayManager _manager;
  final String _label;

  @override
  final String commit;

  @override
  Future<TreeFile?> file(String path, {required int maxBytes}) async => (await readFiles([path], maxBytes: maxBytes))[path];

  @override
  Future<List<String>?> directories(String path) async {
    final answer = await _manager._call('catalog/read', {'label': _label, 'commit': commit, 'paths': const <String>[], 'list': path});
    final listed = answer['directories'];
    return listed is List<Object?> ? [for (final name in listed) '$name'] : null;
  }

  @override
  Future<Map<String, TreeFile?>> readFiles(List<String> paths, {required int maxBytes}) async {
    final answer = await _manager._call('catalog/read', {'label': _label, 'commit': commit, 'paths': paths, 'maxBytes': maxBytes});
    final files = answer['files'] as Map<Object?, Object?>? ?? const {};
    return {
      for (final path in paths)
        path: switch (files[path]) {
          final Map<Object?, Object?> entry when entry['size'] is num => (
            size: (entry['size']! as num).toInt(),
            bytes: entry['data'] is String ? base64Decode(entry['data']! as String) : null,
          ),
          _ => null,
        },
    };
  }
}
