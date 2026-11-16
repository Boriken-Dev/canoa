import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';

import '../session_store.dart';
import '../sources/source_icons.dart';
import 'agent_events.dart' show Choice, ConfigOption;
import 'cache_lifetime.dart';
import 'composer.dart' show ComposerKit;
import 'modes.dart';
import 'session_options.dart';
import 'session_picker.dart';

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
    return InkWell(
      key: const Key('new-session-source'),
      borderRadius: BorderRadius.circular(8),
      onTap: () => unawaited(showSessionPicker(context, store: store, session: session, kit: kit)),
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
}

/// What [role] has waiting in [session]: a choice made and not yet taken by the agent.
PendingChoice? pendingChoice(ChatSession session, OptionRole role) =>
    session.pending.values.where((choice) => choice.role == role).firstOrNull;

/// Chooses [value] for [role] in [session]; null takes a draft's choice back, so its agent's own default holds.
void chooseRole(SessionStore store, ChatSession session, OptionRole role, ConfigOption? option, String? value) {
  final pending = pendingChoice(session, role);
  if (value == null) {
    // Default takes back the choice waiting, and sets nothing.
    if (pending != null) {
      unawaited(store.choose(session, ConfigOption(id: pending.id, label: option?.label ?? 'Choice', current: '', choices: const []), null));
    }
  } else if (option != null) {
    unawaited(store.choose(session, option, value));
  }
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
            tooltip: inRepository ? 'A new git worktree of this workspace, on a branch of its own' : 'The workspace is not in a git repository',
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
  const ModelEffortChooser({super.key, required this.store, required this.session, required this.options, required this.kit, this.seen});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;
  final List<ConfigOption> options;
  final DateTime? seen;

  PendingChoice? _pending(OptionRole role) => pendingChoice(session, role);

  /// What [role] reads in the pill: its choice, else, before the agent starts, what Default is; null when nothing is known.
  String? _part(ConfigOption? option, OptionRole role, {required bool always}) {
    final pending = _pending(role);
    final current = pending?.value ?? (session.isDraft ? null : option?.current);
    if (current != null && current.isNotEmpty) return _labelOf(option, current, role);
    if (!session.isDraft) return null;
    return option?.defaultName ?? (always && (option != null || pending != null) ? 'Default' : null);
  }

  /// The model and effort are chosen in the one picker, which the pill opens.
  void _open(BuildContext context) => unawaited(showSessionPicker(context, store: store, session: session, kit: kit));

  @override
  Widget build(BuildContext context) {
    final model = optionIn(options, OptionRole.model), effort = optionIn(options, OptionRole.effort);
    final pendingModel = _pending(OptionRole.model), pendingEffort = _pending(OptionRole.effort);
    final draft = session.isDraft;
    final known = model != null || effort != null || pendingModel != null || pendingEffort != null;
    if (!known && !draft) return const SizedBox.shrink();
    // A source that could not be started says why in its own chip, and has no list to wait for.
    final source = session.source;
    if (!known && draft && source != null && store.choicesOf(source)?.unknownKind != null) return const SizedBox.shrink();
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
    final connecting = session.status == SessionStatus.connecting;
    return InkWell(
      key: const Key('option-model'),
      borderRadius: BorderRadius.circular(8),
      onTap: connecting ? null : () => _open(context),
      child: Tooltip(
        message: _seenLabel(draft ? seen : null, 'Model and effort'),
        child: ComposerPill(label: label, icon: Icons.memory),
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
          // Canoa's Bypass is a switch: chosen while it is on, it goes off.
          unawaited(value.own && session.bypass ? store.leaveBypass(session) : store.chooseMode(session, value));
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
    final choices = source == null ? null : store.choicesOf(source);
    if (choices == null || choices.cacheLifetime == null || choices.cacheChoices.isEmpty) return const SizedBox.shrink();
    final asked = store.cacheLifetimeOf(session);
    final unavailable = store.cacheUnavailable(session);
    return PopupMenuButton<CacheLifetime?>(
      key: const Key('option-cache'),
      tooltip: 'How long the prompt cache lives',
      enabled: session.status == SessionStatus.draft,
      onSelected: (value) => store.setCacheLifetime(session, value),
      itemBuilder: (context) => [
        PopupMenuItem<CacheLifetime?>(
          key: const Key('cache-unchosen'),
          value: null,
          child: Text(asked.chosen ? 'Not chosen' : 'Not chosen (${choices.cacheLabel(asked.lifetime)})'),
        ),
        CheckedPopupMenuItem<CacheLifetime?>(
          key: Key('cache-${CacheLifetime.harness.id}'),
          value: CacheLifetime.harness,
          checked: asked.chosen && asked.lifetime == CacheLifetime.harness,
          child: Text(CacheLifetime.harness.label),
        ),
        for (final each in choices.cacheChoices)
          CheckedPopupMenuItem<CacheLifetime?>(
            key: Key('cache-${each.lifetime.id}'),
            value: each.lifetime,
            checked: asked.chosen && each.lifetime == asked.lifetime,
            child: Text(each.label),
          ),
        if (unavailable != null)
          PopupMenuItem<CacheLifetime?>(
            key: Key('cache-unavailable-${unavailable.id}'),
            enabled: false,
            value: unavailable,
            child: Text('${unavailable.label}: not offered by this agent, so ${CacheLifetime.harness.label.toLowerCase()} is used'),
          ),
      ],
      child: ComposerPill(label: 'Cache: ${choices.cacheLabel(asked.lifetime)}', icon: Icons.timer_outlined),
    );
  }
}

/// The loadout a draft starts with and, for this session alone, which of its whole overlays are on. A loadout is chosen here by
/// name; what is inside it is edited in Loadouts, and nothing here switches an item inside an overlay.
class OverlaysChooser extends StatelessWidget {
  const OverlaysChooser({super.key, required this.store, required this.session, required this.kit});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;

  @override
  Widget build(BuildContext context) {
    final named = session.loadout != null && kit.loadouts.containsKey(session.loadout) ? session.loadout : kit.defaultLoadout;
    final loadout = kit.loadouts[named];
    final base = loadout?.overlays ?? kit.defaultOverlays ?? kit.overlays;
    final on = session.overlays ?? base.toSet();
    final count = kit.overlays.where(on.contains).length;
    final missing = loadout?.missingOverlays ?? const <String>[];
    final label = '${loadout == null ? 'Overlays' : named!} ($count)${missing.isEmpty ? '' : ', ${missing.length} missing'}';
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
        if (kit.loadouts.isNotEmpty) ...[
          for (final name in kit.loadouts.keys)
            MenuItemButton(
              key: Key('loadout-choice $name'),
              closeOnActivate: false,
              leadingIcon: Icon(name == named ? Icons.radio_button_checked : Icons.radio_button_unchecked, size: 18),
              onPressed: () => store.updateDraft(session, loadout: name),
              child: Text(name == kit.defaultLoadout ? '$name (default)' : name),
            ),
          const Divider(height: 1),
        ],
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
        for (final name in missing)
          MenuItemButton(
            key: Key('overlay-missing $name'),
            leadingIcon: Icon(Icons.warning_amber_outlined, size: 18, color: Theme.of(context).colorScheme.error),
            child: Text('$name is not installed'),
          ),
        if (kit.onLoadout != null) ...[
          const Divider(height: 1),
          MenuItemButton(
            key: const Key('overlays-loadout'),
            leadingIcon: const Icon(Icons.tune, size: 18),
            onPressed: kit.onLoadout,
            child: const Text('Manage loadouts'),
          ),
        ],
      ],
      builder: (context, controller, _) => InkWell(
        key: const Key('overlays'),
        borderRadius: BorderRadius.circular(8),
        onTap: () => controller.isOpen ? controller.close() : controller.open(),
        child: Tooltip(
          message: missing.isEmpty
              ? 'The loadout it starts with, and which of its overlays are on'
              : 'The loadout names ${missing.join(', ')}, which ${missing.length == 1 ? 'is' : 'are'} not installed',
          child: ComposerPill(label: label, icon: missing.isEmpty ? Icons.layers_outlined : Icons.warning_amber_outlined),
        ),
      ),
    );
  }
}
