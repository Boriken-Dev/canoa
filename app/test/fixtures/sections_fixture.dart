import 'dart:convert';

import 'package:canoa/settings/built_in_sections.dart';
import 'package:canoa/settings/host_sections.dart';

/// What `settings/sections` answers for the sections the app draws itself: what `canoa-host`'s own test keeps equal to what the host
/// lists, and the app's fallback.
List<Object?> builtInSectionsJson() => ((jsonDecode(builtInSectionsListing) as Map)['sections']! as List).cast<Object?>();

/// The host's own sections, as an app connected to a host has them.
HostSections builtInSections() => HostSections((method, params) async => {'sections': builtInSectionsJson()})..adopt({'sections': builtInSectionsJson()});
