/// Skills fetched from elsewhere: what Canoa wrote for each beside the overlays, and the checks a reader makes before it
/// delivers one. canoa (`canoa_host::profile`) makes the same checks of the same files, so a skill is delivered by both
/// or by neither.
library;

import 'dart:convert';
import 'dart:typed_data';

import 'package:coqui_aos/coqui_aos.dart' as aos;

import 'aos_support.dart';

/// What one skill may hold, in files and bytes, as the apply path reads it.
const maxSkillFileCount = 1024;
const maxSkillByteCount = 16 * 1024 * 1024;
final aos.Settings skillLimits = treeSettings(maxSkillFileCount, maxSkillByteCount);

/// The directory, beside the installed overlays, holding fetched skills: `<name>/` their files and `<name>.json` their
/// records, both under the skill's name in lower case.
const skillsDirectory = '.skills';

/// The name a skill is kept under: ASCII lower case, since names compare without regard to ASCII case.
String skillKey(String name) => name.replaceAllMapped(RegExp('[A-Z]'), (m) => m[0]!.toLowerCase());

/// Whether [name] can be a directory of a fetched skill and is a skill name.
bool validSkillName(String name) => RegExp(r'^[A-Za-z0-9][A-Za-z0-9_.-]*$').hasMatch(name) && aos.validRelativePath(name);

/// A skill fetched from elsewhere and accepted: where it came from, the tree digest that was fetched, which overlays name
/// it and whether the person chose it themselves, with the files that digest covers.
final class SkillCopy {
  const SkillCopy({required this.name, required this.source, required this.digest, required this.overlays,
      this.ref, this.pin, this.catalog, this.chosen = false, this.files = const {}});

  factory SkillCopy.fromJson(Map<String, dynamic> json, [Map<String, List<int>> files = const {}]) => SkillCopy(
    name: json['name'] as String,
    source: json['source'] as String,
    ref: json['ref'] as String?,
    pin: json['pin'] as String?,
    digest: json['digest'] as String,
    catalog: json['catalog'] as String?,
    overlays: [for (final overlay in (json['overlays'] as List?) ?? const []) '$overlay'],
    chosen: json['chosen'] == true,
    files: files,
  );

  final String name;

  /// The source's identifying text, `location[@ref][#path]`: what a manifest's entry compares with. Never shown.
  final String source;
  final String? ref;

  /// The pin the entry or the catalog gave, if any.
  final String? pin;

  /// The tree digest that was fetched and accepted.
  final String digest;

  /// The label of the catalog that gave the source, when a name was resolved through one.
  final String? catalog;

  /// The overlays that name it, as AOS normalizes their names.
  final List<String> overlays;

  /// Installed by the person's own choice, which no overlay's removal takes away.
  final bool chosen;

  /// The files its digest covers, by relative path.
  final Map<String, List<int>> files;

  Map<String, Object?> toJson() => {
    'name': name,
    'source': source,
    'ref': ?ref,
    'pin': ?pin,
    'digest': digest,
    'catalog': ?catalog,
    'overlays': overlays,
    'chosen': chosen,
  };

  SkillCopy copyWith({List<String>? overlays, bool? chosen}) => SkillCopy(name: name, source: source, ref: ref, pin: pin,
      digest: digest, catalog: catalog, overlays: overlays ?? this.overlays, chosen: chosen ?? this.chosen, files: files);

  /// This record as the JSON text written to `<name>.json`.
  String toText() => '${const JsonEncoder.withIndent('  ').convert(toJson())}\n';

  /// The files as the digest reads them.
  List<(String, Uint8List)> get pairs => [for (final MapEntry(:key, :value) in files.entries) (key, Uint8List.fromList(value))];
}

/// Why the fetched copy [copy] of the skill [entry] names is not delivered for the overlay [identity], or null when it is:
/// the record must list the overlay and name the skill, a source entry must be the very source and pin that were
/// accepted, and the files must still be those whose digest was recorded and be a skill named as the entry names it.
String? skillCopyRefusal(aos.SkillEntry entry, String identity, SkillCopy? copy, {String? configuredPin}) {
  if (copy == null) {
    return entry.origin == 'source'
        ? 'from ${entry.source}, not fetched: install or sync the overlay in Canoa'
        : 'a named skill; no catalog resolved it here: install or sync the overlay in Canoa';
  }
  const stale = 'its fetched copy is not what was accepted: install or sync the overlay in Canoa';
  if (!copy.overlays.contains(aos.normalizeName(identity)) || skillKey(copy.name) != skillKey(entry.name)) return stale;
  if (entry.origin == 'source') {
    if (copy.source != entry.key) return stale;
    if (entry.pin case final pin? when !sameDigest(pin, copy.digest)) return stale;
  }
  if (configuredPin != null && !sameDigest(configuredPin, copy.digest)) return stale;
  try {
    aos.readFetchedSkill(copy.name, copy.pairs, copy.digest, skillLimits);
  } on aos.AosError {
    return stale;
  }
  return null;
}
