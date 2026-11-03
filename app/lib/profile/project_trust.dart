import 'dart:convert';

import 'package:crypto/crypto.dart';

/// The file in a working directory that holds its project profile.
const projectProfileFile = '.canoa/profile.toml';

/// A directory the person trusts, and the project profile it held when they did.
final class TrustedProject {
  const TrustedProject(this.directory, this.sha256, this.trusted);
  final String directory;

  /// The SHA-256 of the profile's bytes, in hex.
  final String sha256;
  final DateTime? trusted;
}

/// Whether a project profile is used.
enum TrustState {
  /// The directory has no project profile.
  none,

  /// The person has not trusted the directory: its profile is ignored.
  untrusted,

  /// The directory is trusted, and its profile changed since: ignored until trusted again.
  changed,

  /// Used.
  trusted,
}

/// The directories whose project profiles the person trusts, per directory, kept by the app and revocable.
/// A trust holds the profile as it was: a profile changed later is ignored until the person trusts it again.
final class ProjectTrust {
  ProjectTrust({required Future<String?> Function() read, required Future<void> Function(String) write,
      bool caseInsensitive = false})
      : _read = read, _write = write, _fold = caseInsensitive;

  final Future<String?> Function() _read;
  final Future<void> Function(String) _write;
  final bool _fold;
  List<TrustedProject> _projects = const [];

  /// The trusted directories, as last loaded or changed.
  List<TrustedProject> get projects => List.unmodifiable(_projects);

  static String digest(List<int> profile) => sha256.convert(profile).toString();

  /// Reads the record; one that cannot be read trusts nothing.
  Future<void> load() async {
    try {
      final text = await _read();
      final json = text == null ? null : jsonDecode(text);
      _projects = [
        if (json case {'projects': final List<dynamic> projects})
          for (final project in projects)
            if (project case {'directory': final String directory, 'sha256': final String digest})
              TrustedProject(directory, digest, DateTime.tryParse('${project['trusted']}')),
      ];
    } on Object {
      _projects = const [];
    }
  }

  bool _same(String a, String b) => _fold ? a.toLowerCase() == b.toLowerCase() : a == b;

  /// [directory] is canonical: links resolved, as the platform names it.
  TrustState state(String directory, List<int>? profile) {
    if (profile == null) return TrustState.none;
    final found = _projects.where((project) => _same(project.directory, directory)).firstOrNull;
    if (found == null) return TrustState.untrusted;
    return found.sha256 == digest(profile) ? TrustState.trusted : TrustState.changed;
  }

  /// Trusts [directory] with [profile] as it is now.
  Future<void> trust(String directory, List<int> profile) async {
    _projects = [
      for (final project in _projects) if (!_same(project.directory, directory)) project,
      TrustedProject(directory, digest(profile), DateTime.now()),
    ];
    await _save();
  }

  /// Stops trusting [directory]; its profile is ignored from the next session on.
  Future<void> revoke(String directory) async {
    _projects = [for (final project in _projects) if (!_same(project.directory, directory)) project];
    await _save();
  }

  Future<void> _save() => _write(const JsonEncoder.withIndent('  ').convert({
    'version': 1,
    'projects': [
      for (final project in _projects)
        {
          'directory': project.directory,
          'sha256': project.sha256,
          if (project.trusted != null) 'trusted': project.trusted!.toUtc().toIso8601String(),
        },
    ],
  }));
}
