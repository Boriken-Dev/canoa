import '../chat/agent_events.dart';
import '../chat/session_options.dart';
import '../providers/provider_catalog.dart';
import 'source_icons.dart';
import 'sources.dart';

/// The modes a source's agent is known to offer before it has run, as its description says: every mode a translator has, Bypass
/// included, or the few a program advertises in `session/new`. Models and efforts are not here: the host asks the source's provider
/// for them (see `SourceChoices`). An agent no loaded provider describes has nothing here, and the composer says its choices come
/// after it starts.
///
/// What the agent then offers wins: a choice made here that it does not offer is reported when it starts, not applied.
///
/// A source is known by its provider ([sourceProvider]), so an entry a served page reaches by address, which names its `harness`,
/// offers them as the host's own source does: the same for every agent, with no key in canoa.cfg to gate Bypass. A program with no
/// harness provider of its own offers them only as a source of this computer.
List<ConfigOption> knownModes(AgentSource source) {
  final provider = providerCatalog.resolve(sourceProvider(source));
  if (provider == null || provider.modes.isEmpty || (!provider.harness && !source.local)) return const [];
  return [
    ConfigOption(
      id: 'mode',
      label: 'Mode',
      current: '',
      category: 'mode',
      choices: [for (final mode in provider.modes) Choice(mode.id, mode.title)],
    ),
  ];
}

/// [first] over [then]: a role [first] has shows its list, any other the one of [then].
List<ConfigOption> overlaid(List<ConfigOption> first, List<ConfigOption> then) => [
  ...first,
  for (final option in then)
    if (!first.any((each) => roleOf(each) == roleOf(option))) option,
];
