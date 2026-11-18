import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import 'profile.dart';
import 'skill_copies.dart';

/// What an overlay holds at one commit: each file, relative to the overlay's directory, with its git blob id.
typedef OverlayFiles = Map<String, String>;

/// An overlay installed on this computer: the commit it was fetched at and the tree digest of the files installed.
final class InstalledOverlay {
  const InstalledOverlay({required this.name, required this.source, required this.commit, required this.files,
      this.digest, this.skipped = const [], this.installed, this.catalog});

  factory InstalledOverlay.fromJson(String name, Map<String, dynamic> json) => InstalledOverlay(
    name: name,
    source: json['source'] as String,
    commit: json['commit'] as String,
    digest: json['digest'] as String?,
    files: {for (final MapEntry(:key, :value) in (json['files'] as Map).entries) '$key': '$value'},
    skipped: [for (final path in (json['skipped'] as List?) ?? const []) '$path'],
    installed: DateTime.tryParse('${json['installed']}'),
    catalog: json['catalog'] as String?,
  );

  final String name;

  /// The source it was installed from, as canoa.cfg wrote it then.
  final String source;
  final String commit;

  /// The AOS tree digest of what was installed; null in a record from before Canoa read AOS overlays.
  final String? digest;
  final OverlayFiles files;

  /// Entries under an excluded name (`.git`, `node_modules`, ...), which are no part of an overlay and never installed.
  final List<String> skipped;
  final DateTime? installed;

  /// The label of the catalog whose store page installed it; null for an overlay named by hand, which the store neither
  /// updates nor removes.
  final String? catalog;

  Map<String, Object?> toJson() => {
    'source': source,
    'commit': commit,
    'digest': ?digest,
    'files': files,
    'skipped': skipped,
    if (installed != null) 'installed': installed!.toUtc().toIso8601String(),
    'catalog': ?catalog,
  };
}

/// What installing or syncing an overlay would change, from the commit installed (none for a first install) to [commit].
final class OverlayUpdate {
  const OverlayUpdate({required this.name, required this.source, required this.from, required this.commit,
      required this.digest, required this.files, required this.skipped, required this.added, required this.changed,
      required this.removed, this.fromSource, this.fromDigest, this.manifest});

  final String name;
  final GitOverlay source;
  final String? from;

  /// The source the installed commit came from, as its record has it.
  final String? fromSource;

  /// The tree digest installed now, as its record has it.
  final String? fromDigest;
  final String commit;

  /// The AOS tree digest of the files at [commit], which applying installs and pins.
  final String digest;
  final OverlayFiles files;
  final List<String> skipped, added, changed, removed;

  /// The overlay's OVERLAY.md, else its overlay.cfg, at [commit], or null when it has none.
  final String? manifest;

  bool get changesNothing =>
      from == commit && fromSource == source.spec && fromDigest == digest && added.isEmpty && changed.isEmpty &&
      removed.isEmpty;
}

/// A git operation or an overlay's content that was refused. Never quotes a file's content.
final class OverlayException implements Exception {
  const OverlayException(this.message);
  final String message;
  @override
  String toString() => message;
}

/// Installs named overlays from git and syncs them, each verified by its AOS tree digest.
abstract interface class OverlayManager {
  /// The overlay installed as [name], or null.
  InstalledOverlay? installed(String name);

  /// The directory an installed overlay's files are in.
  String directoryOf(String name);

  /// Fetches [source] and says what installing it as [name] would change, touching nothing installed. Refuses a tree
  /// whose digest is not the source's pin. [catalog] names the catalog the overlay was chosen from: its source is someone
  /// else's listing, so the host fetches it without the person's git settings, credentials or keys; one the person typed
  /// passes none.
  Future<OverlayUpdate> check(String name, GitOverlay source, {String? catalog});

  /// Installs exactly what [update] showed. [catalog] records that the store installed it from the catalog so labelled; a
  /// record already made for the same source keeps its own when none is given, so a sync from Settings leaves it.
  Future<InstalledOverlay> apply(OverlayUpdate update, {String? catalog});

  /// Removes what [name] installed and fetched.
  Future<void> remove(String name);

  /// Prepares the installed overlay [name]: installs its dependencies into an environment of its own and runs its setup, as the
  /// specification's phases say, outside any agent's session. What it runs is what the plan listed. Throws [OverlayException]
  /// with the step that failed and the end of what it printed.
  Future<void> prepare(String name);

  /// The bytes of every file [update] lists, read from what [check] fetched, without installing anything: what a store
  /// shows an overlay would do is computed from these.
  Future<Map<String, List<int>>> read(OverlayUpdate update);

  /// Fetches the repository of the catalog labelled [label] at [source]'s ref, as [check] fetches an overlay, and gives
  /// its files at that commit to read one by one.
  Future<FetchedTree> fetchCatalog(String label, GitOverlay source);

  /// Removes what [fetchCatalog] fetched for [label].
  Future<void> removeCatalog(String label);

  /// Removes what [check] fetched for [name] when nothing is installed under that name: a preview that was not
  /// installed leaves nothing behind.
  Future<void> discard(String name);

  /// Every skill fetched from elsewhere and accepted, by lower-case name, with its files. Reads nothing of a skill's
  /// files that its record does not name.
  Future<Map<String, SkillCopy>> skillCopies();

  /// The files of the skill at [source], as its directory holds them in git: fetched with none of the person's
  /// credentials, agent or prompts, no redirect to another host followed, nothing checked out and nothing run. [own] says the
  /// person typed the source themselves, which is then fetched as they would from a shell, with their git settings, credentials
  /// and keys. Throws [OverlayException] for a transport that fails, a link, a submodule or more than [skillLimits] allows.
  Future<List<(String, Uint8List)>> fetchSkillFiles(aos.Source source, {bool own = false});

  /// Which skills the git repository [repo] holds at [ref] (its head when null), looked for in [path] of it (all of it when empty)
  /// as `npx skills` looks: each `SKILL.md`'s folder, name and description, and none of a skill's other files. Fetches the
  /// repository one commit deep, with the same settings as [fetchSkillFiles] under [own], and keeps nothing. Throws
  /// [OverlayException] for a transport that fails.
  Future<SkillScan> scanSkills(String repo, {String? ref, String path = '', bool own = false});

  /// The skills a skills directory (skills.sh) lists for [query], most installed first, through the host, which asks the directory
  /// only while `[store] skills_search` is on. A hit names a repository and a skill in it: the directory sends no files, and installing
  /// one reads the repository itself. Throws [OverlayException] when the search is off or the directory cannot be reached.
  Future<List<SkillHit>> searchSkills(String query);

  /// Writes [copy] (its files, then its record) as the skill's copy, replacing any other of that name.
  Future<void> saveSkill(SkillCopy copy);

  /// Removes the skill [name]'s files and record.
  Future<void> removeSkill(String name);
}

/// One result of a skills directory's search: where a skill lives, and how many installs the directory counted for it.
final class SkillHit {
  const SkillHit({required this.source, required this.skillId, required this.name, required this.installs});

  /// The GitHub repository, `owner/repo`.
  final String source;

  /// The skill's name in it, which is the name of its folder.
  final String skillId;
  final String name;
  final int installs;

  /// `owner/repo/skill`, how the directory writes it.
  String get id => '$source/$skillId';

  static final _repository = RegExp(r'^[A-Za-z0-9][A-Za-z0-9_.-]{0,99}/[A-Za-z0-9][A-Za-z0-9_.-]{0,99}$');
  static final _skill = RegExp(r'^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$');

  /// A hit as the host lists it, or null when it is not a skill of a GitHub `owner/repo`: a directory is a third party, so a
  /// result that names a folder, a URL or anything else than a repository and a plain skill name is not followed.
  static SkillHit? fromJson(Object? json) {
    if (json is! Map<Object?, Object?>) return null;
    final source = json['source'], skill = json['skillId'];
    if (source is! String || skill is! String || !_repository.hasMatch(source) || !_skill.hasMatch(skill)) return null;
    final name = json['name'];
    final installs = json['installs'];
    return SkillHit(source: source, skillId: skill, name: name is String && name.isNotEmpty ? name : skill, installs: installs is int && installs > 0 ? installs : 0);
  }
}

/// One skill a source holds, as its `SKILL.md` says.
final class ScannedSkill {
  const ScannedSkill({required this.name, required this.description, required this.path, this.problem, this.source, this.ref});

  /// The name its front matter gives, which an installed skill is kept under.
  final String name;
  final String description;

  /// The skill's folder from the source's root, empty for the root itself.
  final String path;

  /// Why it cannot be installed (a name that is not its folder's), or null.
  final String? problem;

  /// Where a generated listing says the skill is (written as a skill source is), and the ref it pins; null for a skill a scan found,
  /// which is in the source that was scanned.
  final String? source, ref;
}

/// What a source holds at one commit.
final class SkillScan {
  const SkillScan({required this.commit, required this.skills, this.skipped = 0, this.truncated = false});

  /// The commit read: what the person is shown is what installing fetches. Empty for a listing a program or an extension made,
  /// which names the sources its skills are in and reads none.
  final String commit;

  final List<ScannedSkill> skills;

  /// How many `SKILL.md` files were left out: no front matter naming and describing the skill, marked internal, or a repeated name.
  final int skipped;

  /// Whether the source holds more skills than one scan lists.
  final bool truncated;
}

/// One file of a fetched tree: its size, and its bytes unless it was larger than asked.
typedef TreeFile = ({int size, List<int>? bytes});

/// A repository's files at one fetched commit, read from git's objects, never checked out.
abstract interface class FetchedTree {
  String get commit;

  /// The regular file at [path], `/`-separated from the repository's root, or null when there is none (a directory, a
  /// link or a submodule is none). Its bytes are read only when it holds at most [maxBytes].
  Future<TreeFile?> file(String path, {required int maxBytes});

  /// The names of the subdirectories of the directory at [path] (the repository's root for none), or null when there is
  /// no directory there: git keeps none that is empty. A link or a submodule is no subdirectory.
  Future<List<String>?> directories(String path);

  /// [file] for each of [paths], in as few reads as the transport allows: a map holding every path asked, null where
  /// there is no regular file.
  Future<Map<String, TreeFile?>> readFiles(List<String> paths, {required int maxBytes});
}

/// The manager of an app with no host to do the git work: nothing is installed, and anything that would fetch is refused.
class NoOverlays implements OverlayManager {
  const NoOverlays();

  static const _none = OverlayException('Overlays are installed by the host, and this app has none running');

  @override
  InstalledOverlay? installed(String name) => null;

  @override
  String directoryOf(String name) => '';

  @override
  Future<OverlayUpdate> check(String name, GitOverlay source, {String? catalog}) async => throw _none;

  @override
  Future<InstalledOverlay> apply(OverlayUpdate update, {String? catalog}) async => throw _none;

  @override
  Future<void> remove(String name) async => throw _none;

  @override
  Future<void> prepare(String name) async => throw _none;

  @override
  Future<Map<String, List<int>>> read(OverlayUpdate update) async => throw _none;

  @override
  Future<FetchedTree> fetchCatalog(String label, GitOverlay source) async => throw _none;

  @override
  Future<void> removeCatalog(String label) async => throw _none;

  @override
  Future<void> discard(String name) async {}

  @override
  Future<Map<String, SkillCopy>> skillCopies() async => const {};

  @override
  Future<List<(String, Uint8List)>> fetchSkillFiles(aos.Source source, {bool own = false}) async => throw _none;

  @override
  Future<SkillScan> scanSkills(String repo, {String? ref, String path = '', bool own = false}) async => throw _none;

  @override
  Future<List<SkillHit>> searchSkills(String query) async => throw _none;

  @override
  Future<void> saveSkill(SkillCopy copy) async => throw _none;

  @override
  Future<void> removeSkill(String name) async => throw _none;
}
