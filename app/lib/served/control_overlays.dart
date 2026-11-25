import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import '../desktop/control_client.dart';
import '../desktop/host_control.dart';
import '../profile/aos_support.dart' show holdsPassword;
import '../profile/overlays.dart';
import '../profile/profile.dart';
import '../profile/skill_copies.dart';

/// The overlay store's git work done by the host over its control channel (`overlay/*` and `catalog/*`), for a page that cannot run
/// git: the host fetches a source into its own cache, reads what it holds from git's objects without checking anything out,
/// digests it as AOS defines and installs it beside its canoa.cfg. What the store shows and decides stays this app's, over the
/// same [OverlayManager] the desktop's own store implements.
///
/// [installed] answers from what [refresh] last read, since a page cannot read the host's disk in a call that is not a request.
/// Skills fetched from elsewhere go the same way (`skill/*`): the host fetches them with git and keeps their files, and the review
/// of what was fetched stays this app's.
///
/// The desktop app works the same way over its own host's control channel.
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
  Future<OverlayUpdate> check(String name, GitOverlay given, {String? catalog}) async {
    final answer = await _call('overlay/check', {
      'name': name,
      ..._sourceOf(given),
      'pin': ?given.pin,
      'catalog': ?catalog,
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
  Future<Map<String, OverlayAvailability>> updates({bool refresh = false}) async {
    final answer = await _call('overlay/updates', {if (refresh) 'refresh': true});
    final found = <String, OverlayAvailability>{};
    for (final MapEntry(:key, :value) in (answer['overlays'] as Map<Object?, Object?>? ?? const {}).entries) {
      if (key is String) {
        if (OverlayAvailability.fromJson(value) case final each?) found[key] = each;
      }
    }
    return found;
  }

  @override
  Future<void> remove(String name) async {
    await _call('overlay/remove', {'name': name});
    _installed.remove(name);
  }

  @override
  Future<void> prepare(String name) async {
    await _call('overlay/prepare', {'name': name}, const Duration(minutes: 35));
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

  /// What the host holds of each skill fetched from elsewhere. A skill's files are read once for each digest that checks out, so a
  /// refresh asks for the records and for no file it already has.
  final _skillFiles = <String, Map<String, List<int>>>{};

  @override
  Future<Map<String, SkillCopy>> skillCopies() async {
    final answer = await _call('skill/list', const {});
    final out = <String, SkillCopy>{};
    for (final MapEntry(:key, :value) in (answer['skills'] as Map<Object?, Object?>? ?? const {}).entries) {
      if (key is! String || value is! Map) continue;
      try {
        final json = value.cast<String, dynamic>();
        final digest = json['digest'] as String;
        var files = _skillFiles[digest];
        if (files == null) {
          final read = await _call('skill/read', {'name': json['name']});
          files = {
            for (final MapEntry(:key, :value) in (read['files'] as Map<Object?, Object?>? ?? const {}).entries)
              '$key': base64Decode('$value'),
          };
          if (_checksOut(json['name'] as String, files, digest)) _skillFiles[digest] = files;
        }
        out[key] = SkillCopy.fromJson(json, files);
      } on OverlayException {
        rethrow;
      } on Object {
        // A record this app cannot read is no skill it can offer.
      }
    }
    return out;
  }

  static bool _checksOut(String name, Map<String, List<int>> files, String digest) {
    try {
      aos.readFetchedSkill(name, [for (final MapEntry(:key, :value) in files.entries) (key, Uint8List.fromList(value))], digest, skillLimits);
      return true;
    } on aos.AosError {
      return false;
    }
  }

  @override
  Future<List<(String, Uint8List)>> fetchSkillFiles(aos.Source source, {bool own = false}) async {
    if (source.kind != 'git') throw const OverlayException('Canoa fetches skills from git only, and this source is not git');
    if (holdsPassword(source.location)) throw const OverlayException('A skill source holds a password, which none may');
    final answer = await _call('skill/fetch', {
      'repo': source.location,
      'ref': source.reference,
      'path': source.path ?? '',
      if (own) 'own': true,
    });
    final files = answer['files'] as Map<Object?, Object?>? ?? const {};
    return [for (final MapEntry(:key, :value) in files.entries) ('$key', base64Decode('$value'))];
  }

  @override
  Future<SkillScan> scanSkills(String repo, {String? ref, String path = '', bool own = false}) async {
    if (holdsPassword(repo)) throw const OverlayException('A skill source holds a password, which none may');
    final answer = await _call('skill/scan', {'repo': repo, 'ref': ref, 'path': path, if (own) 'own': true});
    final listed = answer['skills'];
    final skills = <ScannedSkill>[
      for (final each in listed is List<Object?> ? listed : const <Object?>[])
        if (each is Map<Object?, Object?> && each['name'] is String && each['path'] is String)
          ScannedSkill(
            name: each['name']! as String,
            description: each['description'] as String? ?? '',
            path: each['path']! as String,
            problem: each['problem'] as String?,
          ),
    ];
    return SkillScan(
      commit: answer['commit']! as String,
      skills: skills,
      skipped: (answer['skipped'] as num?)?.toInt() ?? 0,
      truncated: answer['truncated'] == true,
    );
  }

  @override
  Future<List<SkillHit>> searchSkills(String query) async {
    final answer = await _call('skill/search', {'query': query}, const Duration(seconds: 40));
    final listed = answer['skills'];
    return [
      if (listed is List<Object?>)
        for (final each in listed)
          ?SkillHit.fromJson(each),
    ];
  }

  @override
  Future<void> saveSkill(SkillCopy copy) async {
    await _call('skill/save', {
      ...copy.toJson(),
      'files': {for (final MapEntry(:key, :value) in copy.files.entries) key: base64Encode(value)},
    });
  }

  @override
  Future<void> removeSkill(String name) async {
    await _call('skill/remove', {'name': name});
  }

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
