import 'dart:convert';
import 'dart:io';

import 'package:canoa/accounts/account_models.dart' show AccountInfo;
import 'package:canoa/providers/provider_catalog.dart';

/// What `providers/list` answers for the agents this build bundles: the file `canoa-host`'s own test keeps equal to what the host lists.
Map<String, Object?> bundledProvidersJson() =>
    (jsonDecode(File('test/fixtures/providers_list.json').readAsStringSync()) as Map).cast<String, Object?>();

/// The bundled agents as the app parses them.
List<AgentProvider> bundledProviders() => [
  for (final each in bundledProvidersJson()['providers']! as List) ?AgentProvider.parse(each),
];

/// The readers of an agent's quota a host with every bundled agent offers.
List<UsageReaderOffer> bundledUsageReaders() => [
  for (final each in bundledProvidersJson()['usage_readers']! as List) ?UsageReaderOffer.parse(each),
];

/// The gateways a host with every bundled library offers.
List<GatewayOffer> bundledGateways() => [
  for (final each in bundledProvidersJson()['gateways']! as List) ?GatewayOffer.parse(each),
];

/// The bundled agents of [ids] only, as a host built without the others would list them.
List<AgentProvider> providersOf(Iterable<String> ids) => [for (final each in bundledProviders()) if (ids.contains(each.id)) each];

/// Gives the app the agents a host with every bundled agent lists, as an app connected to one has them.
void listBundledProviders() => providerCatalog.replace(bundledProviders(), usageReaders: bundledUsageReaders(), gateways: bundledGateways());

/// An agent no crate of Canoa's names, with an account provider of its own: what a library that adds one lists.
AgentProvider extraProvider() => AgentProvider.parse({
  'id': 'zed',
  'title': 'Zed',
  'program': 'Zed',
  'vendor': 'zedco',
  'standards': [
    {'id': 'zed-api', 'title': 'Zed API'},
  ],
  'mark': '<svg viewBox="0 0 10 10"><path fill="#336699" d="M0 0H10V10Z"/></svg>',
  'harness': false,
  'accounts': {
    'provider': 'zed_accounts',
    'title': 'Zed accounts',
    'kinds': [
      {'id': 'zed-key', 'title': 'Zed key'},
    ],
    'logins': [
      {'id': 'key', 'title': 'Paste a key', 'produces': 'zed-key'},
    ],
    'usage': <String>[],
  },
  'origin': 'zed-library',
})!;

/// An account of [provider] as the host lists it.
AccountInfo accountOf(String provider, String name) =>
    AccountInfo.parse({'name': name, 'harness': provider, 'kind': '$provider-key', 'id': 'id-$name', 'state': 'signed in'})!;
