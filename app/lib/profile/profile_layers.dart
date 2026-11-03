import 'dart:convert';

import 'profile.dart';
import 'profile_home.dart';
import 'project_trust.dart';

/// Canoa's own layer, lowest of all. It sets nothing: every value comes from a layer the person can see and change.
const builtInProfile = Profile();

/// The file an overlay's directory holds its profile in.
const overlayProfileFile = 'profile.toml';

/// Why a layer is not in the merge, or what to know about one that is.
final class LayerNote {
  const LayerNote(this.layer, this.text);
  final LayerId layer;
  final String text;
}

/// A project's directory and what became of its profile.
final class ProjectLayer {
  const ProjectLayer(this.directory, this.state, [this.profile]);

  /// The working directory, links resolved.
  final String directory;
  final TrustState state;

  /// The profile's bytes, which trusting holds: what the person trusts is what they were shown.
  final List<int>? profile;
}

/// Every layer for one agent and directory, lowest first, and what was left out and why.
final class ProfileLayers {
  const ProfileLayers(this.layers, this.notes, {this.project});
  final List<Layer> layers;
  final List<LayerNote> notes;
  final ProjectLayer? project;

  EffectiveProfile get effective => mergeProfiles(layers);
}

/// The layers for [agent]: built-in, each overlay of [overlays] in order, the [user]'s own, the project's in
/// [workingDirectory] once trusted, the agent entry's from [agents], then [session]. With no [home], as in a web build,
/// overlays and projects are left out and say so.
Future<ProfileLayers> profileLayers({
  required Map<String, OverlaySource> overlays,
  required Profile user,
  required Map<String, Profile> agents,
  required String agent,
  ProfileHome? home,
  String? workingDirectory,
  Profile? session,
}) async {
  final layers = <Layer>[const Layer(LayerId(LayerKind.builtIn), builtInProfile)];
  final notes = <LayerNote>[];
  for (final MapEntry(key: name, value: source) in overlays.entries) {
    final id = LayerId(LayerKind.overlay, name);
    if (home == null) {
      notes.add(LayerNote(id, 'Overlays are read by the desktop app'));
      continue;
    }
    final String directory;
    switch (source) {
      case GitOverlay():
        final installed = home.overlays.installed(name);
        if (installed == null) {
          notes.add(LayerNote(id, 'Not installed: install it to use it'));
          continue;
        }
        if (installed.source != source.spec) {
          notes.add(LayerNote(id, 'Installed from ${installed.source}, not the source Canoa.cfg names: sync it'));
          continue;
        }
        directory = home.overlays.directoryOf(name);
        final commit = installed.commit;
        notes.add(LayerNote(id, 'At commit ${commit.length > 12 ? commit.substring(0, 12) : commit}'));
      case DirOverlay(:final dir):
        final found = home.canonicalDirectory(isAbsolutePath(dir) ? dir : home.join(home.configDir, dir));
        if (found == null) {
          notes.add(LayerNote(id, 'Its directory is not there'));
          continue;
        }
        directory = found;
    }
    final layer = await _read(home, home.join(directory, overlayProfileFile), id, directory, notes);
    if (layer != null) layers.add(layer);
  }
  layers.add(Layer(const LayerId(LayerKind.user), user, dir: home?.configDir));
  ProjectLayer? project;
  if (workingDirectory != null && home != null) {
    const id = LayerId(LayerKind.project);
    final directory = home.canonicalDirectory(workingDirectory);
    if (directory == null) {
      notes.add(const LayerNote(id, 'The working directory is not there'));
    } else {
      final bytes = await home.readBytes(home.join(directory, projectProfileFile));
      await home.trust.load();
      final state = home.trust.state(directory, bytes);
      project = ProjectLayer(directory, state, bytes);
      switch (state) {
        case TrustState.none:
          break;
        case TrustState.untrusted:
          notes.add(const LayerNote(id, 'This directory is not trusted: its profile is ignored'));
        case TrustState.changed:
          notes.add(const LayerNote(id, 'Its profile changed since you trusted it: ignored until you trust it again'));
        case TrustState.trusted:
          final layer = _parse(bytes!, id, directory, notes);
          if (layer != null) layers.add(layer);
      }
    }
  }
  if (agents[agent] case final profile?) {
    layers.add(Layer(LayerId(LayerKind.agent, agent), profile, dir: home?.configDir));
  }
  if (session != null) layers.add(Layer(const LayerId(LayerKind.session), session));
  return ProfileLayers(layers, notes, project: project);
}

Future<Layer?> _read(ProfileHome home, String path, LayerId id, String directory, List<LayerNote> notes) async {
  final bytes = await home.readBytes(path);
  if (bytes == null) {
    notes.add(LayerNote(id, 'Has no $overlayProfileFile: it sets nothing'));
    return null;
  }
  return _parse(bytes, id, directory, notes);
}

Layer? _parse(List<int> bytes, LayerId id, String directory, List<LayerNote> notes) {
  try {
    return Layer(id, Profile.parse(utf8.decode(bytes), id.kind), dir: directory);
  } on ProfileException catch (error) {
    notes.add(LayerNote(id, 'Refused, so left out: $error'));
  } on FormatException {
    notes.add(LayerNote(id, 'Refused, so left out: the profile is not UTF-8'));
  }
  return null;
}
