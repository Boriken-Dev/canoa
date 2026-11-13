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
              leading: Icon(sourceIcon(each), size: 18),
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
          icon: sourceIcon(source),
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

/// One of the agent's choices in a role, as a menu: before the agent starts, what its source offered last time and Default;
/// after, what it offers. A choice waiting for the next turn says so.
class OptionChooser extends StatelessWidget {
  const OptionChooser({super.key, required this.store, required this.session, required this.role, required this.options, this.seen});

  final SessionStore store;
  final ChatSession session;
  final OptionRole role;
  final List<ConfigOption> options;
  final DateTime? seen;

  @override
  Widget build(BuildContext context) {
    final option = optionIn(options, role);
    final name = role == OptionRole.model ? 'Model' : 'Effort';
    final pending = session.pending.values.where((choice) => choice.role == role).firstOrNull;
    final draft = session.isDraft;
    if (option == null && pending == null && !draft) return const SizedBox.shrink();
    // Nothing is known of this agent's list until it runs: say so, rather than offer a lone Default.
    if (option == null && pending == null) {
      return Tooltip(
        key: Key('option-${role.name}'),
        message: 'Its ${name.toLowerCase()} choices are listed once the agent has started',
        child: ComposerPill(label: '$name: after start', icon: role == OptionRole.model ? Icons.memory : Icons.speed, enabled: false),
      );
    }
    final current = pending?.value ?? (draft ? null : option?.current);
    final label = current == null ? (draft ? option?.defaultLabel ?? 'Default' : name) : _labelOf(option, current, role);
    return PopupMenuButton<String?>(
      key: Key('option-${role.name}'),
      tooltip: _seenLabel(draft ? seen : null, name),
      enabled: session.status != SessionStatus.connecting,
      onSelected: (value) {
        if (value == null) {
          // Default takes back the choice waiting, and sets nothing.
          if (pending != null) {
            unawaited(store.choose(session, ConfigOption(id: pending.id, label: name, current: '', choices: const []), null));
          }
        } else if (option != null) {
          unawaited(store.choose(session, option, value));
        }
      },
      itemBuilder: (context) => [
        if (draft) PopupMenuItem<String?>(value: null, child: Text(option?.defaultLabel ?? 'Default')),
        for (final choice in option?.choices ?? const [])
          CheckedPopupMenuItem<String?>(value: choice.value, checked: choice.value == current, child: Text(choiceLabel(role, choice))),
      ],
      child: ComposerPill(
        label: pending != null && session.status == SessionStatus.working ? '$label (next turn)' : label,
        icon: role == OptionRole.model ? Icons.memory : Icons.speed,
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
        if (session.isDraft) const PopupMenuItem<Object?>(value: null, child: Text('Default')),
        // An agent the app knows nothing of lists its modes once it runs; Canoa's own Bypass works with any.
        if (session.isDraft && option == null)
          const PopupMenuItem<Object?>(enabled: false, child: Text('Its other modes: after it has started')),
        for (final entry in entries)
          CheckedPopupMenuItem<Object?>(
            value: entry,
            enabled: entry.selectable,
            checked: entry.value == current,
            child: Text(entry.label),
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
    final on = session.overlays ?? kit.overlays.toSet();
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
