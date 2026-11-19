import 'dart:convert';
import 'dart:io';

import 'package:canoa/settings/host_sections.dart';

/// What `settings/sections` answers for the sections the app draws itself: the file `canoa-host`'s own test keeps equal to what the
/// host lists.
List<Object?> builtInSectionsJson() =>
    ((jsonDecode(File('test/fixtures/settings_sections.json').readAsStringSync()) as Map)['sections']! as List).cast<Object?>();

/// The host's own sections, as an app connected to a host has them.
HostSections builtInSections() => HostSections((method, params) async => {'sections': builtInSectionsJson()})..adopt({'sections': builtInSectionsJson()});

/// Gives Settings the sections a host lists, for a test whose host is never asked.
void listBuiltInSections() => sectionsUntilListed = builtInSectionsJson();
