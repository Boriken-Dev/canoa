import 'dart:convert';

import 'package:canoa/settings/built_in_sections.dart';
import 'package:canoa/settings/host_sections.dart';

/// What `settings/sections` answers for the sections the app draws itself: what `canoa-host`'s own test keeps equal to what the host
/// lists, and the app's fallback.
List<Object?> builtInSectionsJson() => ((jsonDecode(builtInSectionsListing) as Map)['sections']! as List).cast<Object?>();

/// The host's own sections, as an app connected to a host has them.
HostSections builtInSections() => HostSections((method, params) async => {'sections': builtInSectionsJson()})..adopt({'sections': builtInSectionsJson()});

/// The Mail page as the host lists it once the mail extension is loaded: a library the host carries offers it, so its origin is
/// `bundled`, and the application draws it.
Map<String, Object?> mailSectionJson() => {
      'id': 'mail',
      'title': 'Mail',
      'help': null,
      'icon': 'mail',
      'group': 'Agents',
      'under': null,
      'keywords': 'email exchange outlook ews mailbox inbox messages password account add remove test sign in',
      'origin': 'bundled',
      'stored': 'config',
      'schema': null,
      'form': null,
    };

/// The host's own sections and the mail extension's page, as an app connected to a host that carries it has them.
HostSections sectionsWithMail() {
  final listing = {'sections': [...builtInSectionsJson(), mailSectionJson()]};
  return HostSections((method, params) async => listing)..adopt(listing);
}
