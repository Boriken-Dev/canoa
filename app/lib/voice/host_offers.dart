import 'dart:async';

import 'package:flutter/material.dart';

import '../desktop/host_grants.dart';
import '../settings/settings_row.dart';
import '../store/programs_view.dart' show runProgramAction;
import 'host_speech.dart';
import 'provider_form.dart';
import 'speech_backend.dart';

/// Settings > Voice, the speech providers Canoa offers out of the box, which the host of this page or app sets up: a local engine
/// it installs through its Store (the audio is turned into text on the host and goes nowhere else) and the named services it can
/// call with the person's own key. Nothing is installed or added until a button is pressed, and each offer says where the audio goes
/// before it is.
class HostOffersGroup extends StatefulWidget {
  const HostOffersGroup({super.key, required this.catalog, required this.chosen, required this.onChosen});

  final HostSpeechCatalog catalog;

  /// The id of the service `[voice] backend` names now.
  final String chosen;

  /// Called with the id of a provider the person just set up from an offer, which becomes the speech service.
  final void Function(String backendId) onChosen;

  @override
  State<HostOffersGroup> createState() => _HostOffersGroupState();
}

class _HostOffersGroupState extends State<HostOffersGroup> {
  final _model = <String, String>{};
  String? _problem;
  String? _busy;
  String? _adding;

  HostSpeechCatalog get _catalog => widget.catalog;

  String _modelOf(SpeechOffer offer) => _model[offer.id] ?? offer.defaultModel ?? offer.models.firstOrNull?.id ?? '';

  Future<void> _run(String id, Future<void> Function() work) async {
    setState(() {
      _busy = id;
      _problem = null;
    });
    try {
      await work();
    } on Object catch (error) {
      if (mounted) setState(() => _problem = error is SpeechFailure ? error.message : '$error');
    } finally {
      if (mounted) setState(() => _busy = null);
    }
  }

  /// Installs the engine of [offer] with the chosen model through the host's Store, the person confirming the plan, then makes it a
  /// provider and the speech service. With [again], the copy there is taken away first, which is how another model is chosen.
  Future<void> _install(SpeechOffer offer, {bool again = false}) async {
    final programs = _catalog.programs;
    final store = offer.store;
    if (programs == null || store == null) {
      setState(() => _problem = 'This app cannot install programs on its host.');
      return;
    }
    await _run(offer.id, () async {
      await programs.ensure();
      final status = programs.status(store);
      if (status == null) throw const SpeechFailure('The host lists no Store entry for it.');
      if (again && status.hasManaged) {
        await programs.remove(store);
      }
      if (!mounted) return;
      final ran = await runProgramAction(context, programs, programs.status(store) ?? status, 'install', channel: _modelOf(offer));
      if (!ran) return;
      final name = await _catalog.enable(offer.id);
      widget.onChosen(hostSpeechId(name));
    });
  }

  Future<void> _enable(SpeechOffer offer) => _run(offer.id, () async {
    final name = await _catalog.enable(offer.id);
    widget.onChosen(hostSpeechId(name));
  });

  Future<void> _remove(String name) => _run(name, () => _catalog.remove(name));

  Widget _modelChoice(SpeechOffer offer, {bool enabled = true}) => DropdownButton<String>(
    key: Key('offer-model-${offer.id}'),
    value: offer.models.any((m) => m.id == _modelOf(offer)) ? _modelOf(offer) : null,
    isDense: true,
    isExpanded: true,
    items: [for (final m in offer.models) DropdownMenuItem(value: m.id, child: Text(m.label, overflow: TextOverflow.ellipsis))],
    onChanged: enabled ? (id) => setState(() => _model[offer.id] = id!) : null,
  );

  Widget _row(SpeechOffer offer) {
    final scheme = Theme.of(context).colorScheme;
    final made = offer.provider;
    final busy = _busy == offer.id;
    final setup = _catalog.may(HostGrants.voiceSetup);
    final installs = _catalog.may(HostGrants.programs);
    final inUse = made != null && widget.chosen == hostSpeechId(made);
    final where = '${offer.local ? 'On the host. ' : 'A named service. '}${offer.where}';
    Widget action;
    String? why;
    if (made != null) {
      action = Row(mainAxisSize: MainAxisSize.min, children: [
        if (inUse)
          const Chip(key: Key('offer-in-use'), label: Text('In use'), visualDensity: VisualDensity.compact)
        else
          OutlinedButton(key: Key('offer-use-${offer.id}'), onPressed: () => widget.onChosen(hostSpeechId(made)), child: const Text('Use it')),
        IconButton(
          key: Key('offer-remove-${offer.id}'),
          tooltip: 'Remove $made',
          icon: const Icon(Icons.delete_outline),
          onPressed: busy || !setup ? null : () => unawaited(_remove(made)),
        ),
      ]);
    } else if (offer.local && offer.installed) {
      why = setup ? null : 'This host has not given this key the voice.setup grant.';
      action = FilledButton(key: Key('offer-enable-${offer.id}'), onPressed: busy || !setup ? null : () => unawaited(_enable(offer)), child: const Text('Set up'));
    } else if (offer.local) {
      why = !installs
          ? 'This host has not given this key the store.programs grant, which installing a program needs.'
          : !setup
              ? 'This host has not given this key the voice.setup grant.'
              : null;
      action = FilledButton(
        key: Key('offer-install-${offer.id}'),
        onPressed: busy || why != null ? null : () => unawaited(_install(offer)),
        child: const Text('Install'),
      );
    } else {
      why = setup ? null : 'This host has not given this key the voice.setup grant.';
      action = FilledButton(
        key: Key('offer-add-${offer.id}'),
        onPressed: why != null ? null : () => setState(() => _adding = _adding == offer.id ? null : offer.id),
        child: Text(_adding == offer.id ? 'Close' : 'Add'),
      );
    }
    final kind = offer.kind == null ? null : _catalog.types.where((t) => t.id == offer.kind).firstOrNull;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      SettingsRow(
        key: Key('offer-${offer.id}'),
        title: offer.title,
        description: '${offer.summary}\n$where${offer.license == null ? '' : ' Licence: ${offer.license}.'}'
            '${offer.installed && offer.version != null ? ' Installed: ${offer.version}.' : ''}',
        control: busy ? const SizedBox.square(dimension: 20, child: CircularProgressIndicator(strokeWidth: 2)) : action,
        below: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          if (offer.models.isNotEmpty && made == null && offer.local && !offer.installed)
            Row(children: [const Text('Model  '), Flexible(child: _modelChoice(offer))]),
          if (offer.local && made != null && offer.models.isNotEmpty && setup && installs)
            Row(children: [
              const Text('Another model  '),
              Flexible(child: _modelChoice(offer)),
              TextButton(key: Key('offer-switch-${offer.id}'), onPressed: busy ? null : () => unawaited(_install(offer, again: true)), child: const Text('Install it')),
            ]),
          if (why != null) Text(why, key: Key('offer-why-${offer.id}'), style: TextStyle(color: scheme.onSurfaceVariant)),
        ]),
      ),
      if (_adding == offer.id && kind != null && offer.kind != null)
        ProviderSettingsForm(
          key: Key('offer-form-${offer.id}'),
          catalog: _catalog,
          kind: kind,
          name: offer.id,
          saveLabel: 'Add ${offer.title}',
          provider: HostSpeechProvider(
            name: offer.id,
            kind: offer.kind!,
            title: offer.title,
            description: offer.summary,
            settings: offer.settings,
            secretsSet: const {},
            partials: false,
          ),
          onSaved: () => setState(() => _adding = null),
        ),
    ]);
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: _catalog,
    builder: (context, _) {
      final offers = _catalog.offers;
      if (offers.isEmpty) return const SizedBox.shrink();
      return SettingsGroup(title: 'Offered by Canoa', children: [
        for (final offer in offers) _row(offer),
        if (_problem case final problem?)
          Padding(
            padding: const EdgeInsets.all(8),
            child: Text(problem, key: const Key('offer-problem'), style: TextStyle(color: Theme.of(context).colorScheme.error)),
          ),
      ]);
    },
  );
}
