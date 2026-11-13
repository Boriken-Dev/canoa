import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../session_store.dart';
import '../sources/source_icons.dart';
import '../sources/sources.dart';
import 'agent_events.dart' show Choice, ConfigOption;
import 'cache_lifetime.dart';
import 'composer.dart' show ComposerKit;
import 'modes.dart';
import 'session_options.dart';

/// Where a draft's agent comes from: the sources Settings has, each saying what it needs when it cannot run here.
class SourceChooser extends StatelessWidget {
  const SourceChooser({super.key, required this.store, required this.session, required this.kit});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  Widget build(BuildContext context) {
    final source = session.source;
    final problem = source == null ? null : kit.checks[source]?.describe(source.displayName);
    return PopupMenuButton<Object>(
      key: const Key('new-session-source'),
      tooltip: 'Where its agent comes from',
      onSelected: (chosen) => chosen is AgentSource ? store.updateDraft(session, source: chosen) : kit.onSourceSettings?.call(),
      itemBuilder: (context) => [
        for (final each in kit.sources)
          PopupMenuItem(
            key: Key('source ${each.label}'),
            value: each,
            child: ListTile(
              dense: true,
              contentPadding: EdgeInsets.zero,
              leading: SourceMark.of(each, size: 18),
              title: Text(each.displayName),
              subtitle: switch (kit.checks[each]) {
                final check? when !check.ready => Text('Needs ${_and(check.missing.isEmpty ? ['setting up'] : check.missing)}',
                    style: TextStyle(color: Theme.of(context).colorScheme.error)),
                _ => null,
              },
            ),
          ),
        if (kit.onSourceSettings != null) ...[
          const PopupMenuDivider(),
          const PopupMenuItem(key: Key('source-settings'), value: #settings, child: Text('Sources in Settings…')),
        ],
      ],
      child: Tooltip(
        message: problem ?? 'Where its agent comes from',
        child: ComposerPill(
          label: source?.displayName ?? 'Choose a source',
          leading: SourceMark.of(source),
          danger: problem != null || source == null,
        ),
      ),
    );
  }

  static String _and(List<String> words) =>
      words.length < 2 ? words.join() : '${words.sublist(0, words.length - 1).join(', ')} and ${words.last}';
}

/// Whether a draft starts in its folder or in a new git worktree of it: Worktree only for a folder in a git repository.
class PlaceToggle extends StatelessWidget {
  const PlaceToggle({super.key, required this.store, required this.session, required this.kit});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  Widget build(BuildContext context) {
    final cwd = session.cwd;
    final inRepository = cwd != null && (kit.inGitRepository?.call(cwd) ?? false);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 4),
      child: SegmentedButton<bool>(
        key: const Key('new-session-place'),
        style: const ButtonStyle(visualDensity: VisualDensity(horizontal: -4, vertical: -4), tapTargetSize: MaterialTapTargetSize.shrinkWrap),
        showSelectedIcon: false,
        segments: [
          const ButtonSegment(value: false, label: Text('Local', style: TextStyle(fontSize: 12.5))),
          ButtonSegment(
            value: true,
            enabled: inRepository,
            tooltip: inRepository ? 'A new git worktree of this folder, on a branch of its own' : 'The folder is not in a git repository',
            label: const Text('Worktree', style: TextStyle(fontSize: 12.5)),
          ),
        ],
        selected: {session.wantsWorktree && inRepository},
        onSelectionChanged: (chosen) => store.updateDraft(session, worktree: chosen.first),
      ),
    );
  }
}

/// The label of [value] in [option], or [value] itself.
String _labelOf(ConfigOption? option, String value, OptionRole role) {
  final choice = option?.choices.where((choice) => choice.value == value).firstOrNull ?? Choice(value, value);
  return choiceLabel(role, choice);
}

String _seenLabel(DateTime? seen, String label) {
  if (seen == null) return label;
  final local = seen.toLocal();
  String two(int n) => n.toString().padLeft(2, '0');
  return '$label, as its agent offered on ${local.year}-${two(local.month)}-${two(local.day)} ${two(local.hour)}:${two(local.minute)}';
}

/// The agent's model and effort as one pill ("Opus · High"), whose menu lists the models and has the efforts in a submenu. Before
/// the agent starts it lists what its source offered last time and says what Default is where the source knows it; after, what the
/// agent offers. A choice waiting for the next turn says so.
class ModelEffortChooser extends StatelessWidget {
  const ModelEffortChooser({super.key, required this.store, required this.session, required this.options, this.seen});

  final SessionStore store;
  final ChatSession session;
  final List<ConfigOption> options;
  final DateTime? seen;

  PendingChoice? _pending(OptionRole role) => session.pending.values.where((choice) => choice.role == role).firstOrNull;

  /// What [role] reads in the pill: its choice, else, before the agent starts, what Default is; null when nothing is known.
  String? _part(ConfigOption? option, OptionRole role, {required bool always}) {
    final pending = _pending(role);
    final current = pending?.value ?? (session.isDraft ? null : option?.current);
    if (current != null && current.isNotEmpty) return _labelOf(option, current, role);
    if (!session.isDraft) return null;
    return option?.defaultName ?? (always && (option != null || pending != null) ? 'Default' : null);
  }

  void _choose(OptionRole role, ConfigOption? option, String? value) {
    final pending = _pending(role);
    if (value == null) {
      // Default takes back the choice waiting, and sets nothing.
      if (pending != null) {
        unawaited(store.choose(session, ConfigOption(id: pending.id, label: option?.label ?? 'Choice', current: '', choices: const []), null));
      }
    } else if (option != null) {
      unawaited(store.choose(session, option, value));
    }
  }

  Widget _item(ConfigOption? option, OptionRole role, String? value, String label, String? current, String kind) => MenuItemButton(
    key: ValueKey('$kind=${value ?? ''}'),
    leadingIcon: Icon(Icons.check, size: 18, color: value == current ? null : Colors.transparent),
    onPressed: () => _choose(role, option, value),
    child: Text(label),
  );

  @override
  Widget build(BuildContext context) {
    final model = optionIn(options, OptionRole.model), effort = optionIn(options, OptionRole.effort);
    final pendingModel = _pending(OptionRole.model), pendingEffort = _pending(OptionRole.effort);
    final draft = session.isDraft;
    final known = model != null || effort != null || pendingModel != null || pendingEffort != null;
    if (!known && !draft) return const SizedBox.shrink();
    // Nothing is known of this agent's list until it runs: say so, rather than offer a lone Default.
    if (!known) {
      return const Tooltip(
        key: Key('option-model'),
        message: 'Its model and effort choices are listed once the agent has started',
        child: ComposerPill(label: 'Model: after start', icon: Icons.memory, enabled: false),
      );
    }
    final parts = [
      _part(model, OptionRole.model, always: true),
      _part(effort, OptionRole.effort, always: false),
    ].whereType<String>().toList();
    final waiting = (pendingModel != null || pendingEffort != null) && session.status == SessionStatus.working;
    final label = '${parts.isEmpty ? (draft ? 'Default' : 'Model') : parts.join(' · ')}${waiting ? ' (next turn)' : ''}';
    final currentModel = pendingModel?.value ?? (draft ? null : model?.current);
    final currentEffort = pendingEffort?.value ?? (draft ? null : effort?.current);
    final connecting = session.status == SessionStatus.connecting;
    return MenuAnchor(
      menuChildren: [
        if (draft && (model != null || pendingModel != null))
          _item(model, OptionRole.model, null, model?.defaultLabel ?? 'Default', currentModel, 'model-choice'),
        for (final choice in model?.choices ?? const <Choice>[])
          _item(model, OptionRole.model, choice.value, choiceLabel(OptionRole.model, choice), currentModel, 'model-choice'),
        if (effort != null || pendingEffort != null) ...[
          const Divider(height: 1),
          SubmenuButton(
            key: const Key('option-effort'),
            menuChildren: [
              if (draft) _item(effort, OptionRole.effort, null, effort?.defaultLabel ?? 'Default', currentEffort, 'effort-choice'),
              for (final choice in effort?.choices ?? const <Choice>[])
                _item(effort, OptionRole.effort, choice.value, choiceLabel(OptionRole.effort, choice), currentEffort, 'effort-choice'),
            ],
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              const Text('Effort'),
              const SizedBox(width: 24),
              Text(_part(effort, OptionRole.effort, always: true) ?? '', style: TextStyle(color: Theme.of(context).hintColor)),
            ]),
          ),
        ],
      ],
      builder: (context, controller, _) => InkWell(
        key: const Key('option-model'),
        borderRadius: BorderRadius.circular(8),
        onTap: connecting ? null : () => controller.isOpen ? controller.close() : controller.open(),
        child: Tooltip(
          message: _seenLabel(draft ? seen : null, 'Model and effort'),
          child: ComposerPill(label: label, icon: Icons.memory),
        ),
      ),
    );
  }
}

/// The mode the session is in, at the composer's bottom left: the agent's own modes, and Bypass beside them where the agent has
/// none that asks for nothing. Choosing one asks nothing; Shift+Tab in the message cycles them. While Bypass is on it shows in
/// the accent of the scheme as a status.
class ModeChooser extends StatelessWidget {
  const ModeChooser({super.key, required this.store, required this.session, required this.options, this.seen});

  final SessionStore store;
  final ChatSession session;
  final List<ConfigOption> options;
  final DateTime? seen;

  @override
  Widget build(BuildContext context) {
    if (session.status == SessionStatus.preview) return const SizedBox.shrink();
    final option = modeOptionIn(options);
    final entries = modeEntries(option);
    final current = store.modeOf(session);
    final bypass = store.inBypass(session);
    final pending = session.pending.values.any((choice) => choice.role == OptionRole.mode);
    final shown = entries.where((entry) => entry.value == current).firstOrNull;
    // Canoa's own Bypass sits on the agent's mode, which is said beside it.
    final under = session.bypass ? entries.where((entry) => entry.value == store.underlyingMode(session)).firstOrNull : null;
    final label = under == null ? (shown?.label ?? (current == null ? null : choiceLabel(OptionRole.mode, Choice(current, current))) ?? (session.isDraft ? 'Default' : 'Mode')) : 'Bypass over ${under.label}';
    return PopupMenuButton<Object?>(
      key: const Key('option-mode'),
      tooltip: _seenLabel(session.isDraft ? seen : null, 'Mode (Shift+Tab changes it)'),
      enabled: store.canChangeMode(session) || session.isDraft,
      onSelected: (value) {
        if (value is ModeEntry) {
          unawaited(store.chooseMode(session, value));
        } else if (value == null) {
          store.clearMode(session);
        }
      },
      itemBuilder: (context) => [
        PopupMenuItem<Object?>(
          key: const Key('mode-header'),
          enabled: false,
          height: 28,
          child: Text('Permissions \u00b7 this session', style: Theme.of(context).textTheme.labelMedium),
        ),
        if (session.isDraft) const PopupMenuItem<Object?>(value: null, child: Text('Default')),
        // An agent the app knows nothing of lists its modes once it runs; Canoa's own Bypass works with any.
        if (session.isDraft && option == null)
          const PopupMenuItem<Object?>(enabled: false, child: Text('Its other modes: after it has started')),
        for (final entry in entries)
          CheckedPopupMenuItem<Object?>(
            value: entry,
            enabled: entry.selectable,
            checked: entry.value == current,
            child: entry.description == null
                ? Text(entry.label)
                : Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(entry.label),
                      Text(entry.description!, style: Theme.of(context).textTheme.bodySmall?.copyWith(color: Theme.of(context).hintColor)),
                    ],
                  ),
          ),
      ],
      child: ComposerPill(
        label: pending && session.status == SessionStatus.working ? '$label (next turn)' : label,
        icon: bypass ? Icons.fast_forward_outlined : Icons.tune,
        status: bypass,
      ),
    );
  }
}

/// The agent's other choices, behind one control.
class MoreChooser extends StatelessWidget {
  const MoreChooser({super.key, required this.store, required this.session, required this.options});

  final SessionStore store;
  final ChatSession session;
  final List<ConfigOption> options;

  @override
  Widget build(BuildContext context) {
    final others = options.where((option) => roleOf(option) == OptionRole.other).toList();
    return PopupMenuButton<(ConfigOption, String)>(
      key: const Key('option-more'),
      tooltip: 'More of the agent\'s choices',
      onSelected: (chosen) => unawaited(store.choose(session, chosen.$1, chosen.$2)),
      itemBuilder: (context) => [
        for (final option in others) ...[
          PopupMenuItem(enabled: false, height: 28, child: Text(option.label, style: const TextStyle(fontSize: 12))),
          for (final choice in option.choices)
            CheckedPopupMenuItem(
              value: (option, choice.value),
              checked: choice.value == (session.pending[option.id]?.value ?? (session.isDraft ? null : option.current)),
              child: Text(choice.label),
            ),
        ],
      ],
      child: const ComposerPill(label: 'More', icon: Icons.more_horiz),
    );
  }
}

/// Which overlays a draft starts with; once it has started, what its profile gave it.
/// How long a new session's prompt cache lives, for one session: shown only where the source's provider said what it does with
/// the choice, since a source that cannot say has no such control to set. Its sentence is under the composer.
class CacheChooser extends StatelessWidget {
  const CacheChooser({super.key, required this.store, required this.session});

  final SessionStore store;
  final ChatSession session;

  @override
  Widget build(BuildContext context) {
    final source = session.source;
    if (source == null || store.choicesOf(source)?.cacheLifetime == null) return const SizedBox.shrink();
    final asked = store.cacheLifetimeOf(session);
    return PopupMenuButton<CacheLifetime?>(
      key: const Key('option-cache'),
      tooltip: 'How long the prompt cache lives',
      enabled: session.status == SessionStatus.draft,
      onSelected: (value) => store.setCacheLifetime(session, value),
      itemBuilder: (context) => [
        PopupMenuItem<CacheLifetime?>(
          key: const Key('cache-unchosen'),
          value: null,
          child: Text(asked.chosen ? 'Not chosen' : 'Not chosen (${asked.lifetime.label})'),
        ),
        for (final each in CacheLifetime.values)
          CheckedPopupMenuItem<CacheLifetime?>(
            key: Key('cache-${each.id}'),
            value: each,
            checked: asked.chosen && each == asked.lifetime,
            child: Text('${each.label}, ${each.about}'),
          ),
      ],
      child: ComposerPill(label: 'Cache: ${asked.lifetime.label}', icon: Icons.timer_outlined),
    );
  }
}

class OverlaysChooser extends StatelessWidget {
  const OverlaysChooser({super.key, required this.store, required this.session, required this.kit});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  Widget build(BuildContext context) {
    final on = session.overlays ?? (kit.defaultOverlays ?? kit.overlays).toSet();
    final count = kit.overlays.where(on.contains).length;
    if (!session.isDraft) {
      return InkWell(
        key: const Key('overlays'),
        borderRadius: BorderRadius.circular(8),
        onTap: kit.onProfile == null ? null : () => kit.onProfile!(session),
        child: Tooltip(
          message: 'What its profile gave it, set when it started',
          child: ComposerPill(label: 'Overlays${session.overlays == null ? '' : ' ($count)'}', icon: Icons.layers_outlined, enabled: false),
        ),
      );
    }
    return MenuAnchor(
      menuChildren: [
        for (final name in kit.overlays)
          CheckboxMenuButton(
            key: Key('overlay $name'),
            value: on.contains(name),
            closeOnActivate: false,
            onChanged: session.status != SessionStatus.draft
                ? null
                : (value) {
                    final next = {...on};
                    value == true ? next.add(name) : next.remove(name);
                    store.updateDraft(session, overlays: next);
                  },
            child: Text(name),
          ),
        if (kit.onLoadout != null) ...[
          const Divider(height: 1),
          MenuItemButton(
            key: const Key('overlays-loadout'),
            leadingIcon: const Icon(Icons.tune, size: 18),
            onPressed: kit.onLoadout,
            child: const Text('Manage in Loadout'),
          ),
        ],
      ],
      builder: (context, controller, _) => InkWell(
        key: const Key('overlays'),
        borderRadius: BorderRadius.circular(8),
        onTap: () => controller.isOpen ? controller.close() : controller.open(),
        child: Tooltip(
          message: 'The overlays it starts with',
          child: ComposerPill(label: 'Overlays ($count)', icon: Icons.layers_outlined),
        ),
      ),
    );
  }
}
