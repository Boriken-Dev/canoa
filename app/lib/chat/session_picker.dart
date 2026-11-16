import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../accounts/account_models.dart';
import '../accounts/accounts_controller.dart';
import '../accounts/usage_view.dart' show windowSummary;
import '../session_store.dart';
import '../sources/source_icons.dart';
import '../sources/sources.dart';
import 'agent_events.dart' show Choice, ConfigOption;
import 'composer.dart' show ComposerKit;
import 'composer_controls.dart' show chooseRole, pendingChoice;
import 'modes.dart';
import 'session_options.dart';
import 'source_trouble.dart';

/// Below this width the picker fills the window instead of being a panel over the composer.
const double pickerPhoneWidth = 640;

/// Opens the one picker for a conversation's agent, account, model, effort and mode. Everything in it comes from what the app
/// already holds (the source's listed models, the accounts' state, the agent's own options): nothing is asked of the host while
/// it is open or as the search is typed.
Future<void> showSessionPicker(
  BuildContext context, {
  required SessionStore store,
  required ChatSession session,
  required ComposerKit kit,
}) {
  // The width the dialog is laid out in decides, so a narrow window gets the full-window form.
  return showDialog<void>(
    context: context,
    builder: (context) => LayoutBuilder(
      builder: (context, box) => box.maxWidth < pickerPhoneWidth
          ? Dialog.fullscreen(child: SafeArea(child: SessionPicker(store: store, session: session, kit: kit, phone: true)))
          : Dialog(
              alignment: Alignment.bottomCenter,
              insetPadding: const EdgeInsets.all(24),
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 780, maxHeight: 520),
                child: SessionPicker(store: store, session: session, kit: kit, phone: false),
              ),
            ),
    ),
  );
}

/// One row of the model list: a model of the chosen agent, or, under Favourites, of any agent whose models are held.
class _ModelRow {
  const _ModelRow({required this.value, required this.label, this.detail, this.source, this.number});

  /// Null for the row that stands for the agent's own default.
  final String? value;
  final String label;
  final String? detail;

  /// The agent the model belongs to, when it is not the chosen one.
  final AgentSource? source;
  final int? number;

  _ModelRow numbered(int number) => _ModelRow(value: value, label: label, detail: detail, source: source, number: number);
}

class SessionPicker extends StatefulWidget {
  const SessionPicker({super.key, required this.store, required this.session, required this.kit, required this.phone});

  final SessionStore store;
  final ChatSession session;
  final ComposerKit kit;
  final bool phone;

  @override
  State<SessionPicker> createState() => _SessionPickerState();
}

class _SessionPickerState extends State<SessionPicker> {
  final _search = TextEditingController();
  final _focus = FocusNode();
  bool _stars = false;
  int _highlight = 0;

  SessionStore get store => widget.store;
  ChatSession get session => widget.session;
  ComposerKit get kit => widget.kit;
  bool get draft => session.isDraft;

  @override
  void initState() {
    super.initState();
    _search.addListener(() => setState(() => _highlight = 0));
  }

  @override
  void dispose() {
    _search.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _close() => Navigator.of(context).pop();

  List<_ModelRow> _rows(List<ConfigOption> options) {
    final query = _search.text.trim().toLowerCase();
    bool matches(_ModelRow row) =>
        query.isEmpty || row.label.toLowerCase().contains(query) || (row.value ?? '').toLowerCase().contains(query) || (row.detail ?? '').toLowerCase().contains(query);
    final rows = <_ModelRow>[];
    if (_stars) {
      for (final (label, id) in store.ledger?.favourites ?? const <(String, String)>[]) {
        final source = kit.sources.where((each) => each.label == label).firstOrNull;
        final model = source == null ? null : store.choicesOf(source)?.models.where((each) => each.id == id).firstOrNull;
        if (source == null || model == null) continue;
        rows.add(_ModelRow(value: id, label: model.name, detail: '${source.displayName}${model.description == null ? '' : ' · ${model.description}'}', source: source));
      }
    } else {
      final source = session.source;
      final option = optionIn(options, OptionRole.model);
      if (draft && (option != null || pendingChoice(session, OptionRole.model) != null)) {
        rows.add(_ModelRow(value: null, label: option?.defaultLabel ?? 'Default'));
      }
      final models = source == null ? null : store.choicesOf(source)?.models;
      for (final choice in option?.choices ?? const <Choice>[]) {
        final listed = models?.where((each) => each.id == choice.value).firstOrNull;
        rows.add(_ModelRow(value: choice.value, label: choiceLabel(OptionRole.model, choice), detail: listed?.description));
      }
    }
    var number = 0;
    return [
      for (final row in rows)
        if (row.value == null ? query.isEmpty : matches(row)) row.value == null ? row : (++number <= 9 ? row.numbered(number) : row),
    ];
  }

  void _pickModel(_ModelRow row) {
    final source = row.source;
    if (source != null && source != session.source) {
      if (!draft) return;
      store.updateDraft(session, source: source);
    }
    chooseRole(store, session, OptionRole.model, optionIn(store.optionsOf(session), OptionRole.model), row.value);
  }

  void _pickSource(AgentSource source) {
    if (!draft || source == session.source) return;
    store.updateDraft(session, source: source);
    setState(() => _stars = false);
  }

  void _pickNumber(int n, List<_ModelRow> rows) {
    final row = rows.where((each) => each.number == n).firstOrNull;
    if (row == null) return;
    _pickModel(row);
    _close();
  }

  @override
  Widget build(BuildContext context) {
    final listenables = <Listenable>[store, ?store.ledger, ?kit.accounts];
    return ListenableBuilder(
      listenable: Listenable.merge(listenables),
      builder: (context, _) {
        final options = store.optionsOf(session);
        final rows = _rows(options);
        final bindings = <ShortcutActivator, VoidCallback>{
          for (var n = 1; n <= 9; n++) ...{
            SingleActivator(LogicalKeyboardKey(LogicalKeyboardKey.digit0.keyId + n), control: true): () => _pickNumber(n, rows),
            SingleActivator(LogicalKeyboardKey(LogicalKeyboardKey.digit0.keyId + n), meta: true): () => _pickNumber(n, rows),
          },
          const SingleActivator(LogicalKeyboardKey.arrowDown): () => setState(() => _highlight = rows.isEmpty ? 0 : (_highlight + 1) % rows.length),
          const SingleActivator(LogicalKeyboardKey.arrowUp): () => setState(() => _highlight = rows.isEmpty ? 0 : (_highlight - 1 + rows.length) % rows.length),
        };
        final body = widget.phone ? _phone(context, options, rows) : _wide(context, options, rows);
        return CallbackShortcuts(
          bindings: bindings,
          child: Material(
            key: const Key('session-picker'),
            color: Colors.transparent,
            child: body,
          ),
        );
      },
    );
  }

  // ---- layouts

  Widget _wide(BuildContext context, List<ConfigOption> options, List<_ModelRow> rows) {
    final look = context.chatLook;
    return Column(mainAxisSize: MainAxisSize.min, children: [
      Flexible(
        child: Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          SizedBox(width: 176, child: _rail(context, vertical: true)),
          VerticalDivider(width: 1, color: look.border),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.fromLTRB(12, 8, 12, 0),
              child: _middle(context, options, rows),
            ),
          ),
          VerticalDivider(width: 1, color: look.border),
          SizedBox(width: 188, child: Padding(padding: const EdgeInsets.fromLTRB(12, 8, 12, 0), child: _effort(context, options, wrap: false))),
        ]),
      ),
      Divider(height: 1, color: look.border),
      Padding(padding: const EdgeInsets.fromLTRB(12, 6, 12, 8), child: _bottom(context, options)),
    ]);
  }

  Widget _phone(BuildContext context, List<ConfigOption> options, List<_ModelRow> rows) {
    final look = context.chatLook;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Padding(
        padding: const EdgeInsets.fromLTRB(16, 4, 4, 0),
        child: Row(children: [
          Expanded(child: Text('Agent, model and effort', style: Theme.of(context).textTheme.titleMedium)),
          IconButton(key: const Key('picker-close'), tooltip: 'Close', icon: const Icon(Icons.close), onPressed: _close),
        ]),
      ),
      SizedBox(height: 56, child: _rail(context, vertical: false)),
      Divider(height: 1, color: look.border),
      Expanded(child: Padding(padding: const EdgeInsets.symmetric(horizontal: 16), child: _middle(context, options, rows))),
      Padding(padding: const EdgeInsets.symmetric(horizontal: 16), child: _effort(context, options, wrap: true)),
      Divider(height: 1, color: look.border),
      Padding(padding: const EdgeInsets.fromLTRB(16, 8, 16, 12), child: _bottom(context, options)),
    ]);
  }

  // ---- the agents

  Widget _rail(BuildContext context, {required bool vertical}) {
    final look = context.chatLook;
    final tiles = <Widget>[
      _railTile(
        context,
        key: const Key('picker-favourites'),
        leading: Icon(_stars ? Icons.star : Icons.star_border, size: 18, color: _stars ? look.accent : look.muted),
        title: 'Favourites',
        selected: _stars,
        onTap: () => setState(() => _stars = !_stars),
      ),
      for (final each in kit.sources) _sourceTile(context, each),
    ];
    final settings = kit.onSourceSettings;
    final link = settings == null
        ? null
        : TextButton.icon(
            key: const Key('source-settings'),
            onPressed: () {
              _close();
              settings();
            },
            icon: const Icon(Icons.settings_outlined, size: 16),
            label: const Text('Sources in Settings…', style: TextStyle(fontSize: 12.5)),
          );
    if (!vertical) {
      return SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        child: Row(children: [...tiles, ?link]),
      );
    }
    return ListView(padding: const EdgeInsets.symmetric(vertical: 8), children: [...tiles, if (link != null) Padding(padding: const EdgeInsets.only(top: 4), child: link)]);
  }

  Widget _sourceTile(BuildContext context, AgentSource each) {
    final look = context.chatLook;
    final choices = store.choicesOf(each);
    final trouble = sourceTrouble(each, choices, accounts: kit.accounts);
    final check = kit.checks[each];
    final needs = check != null && !check.ready ? 'Needs ${_and(check.missing.isEmpty ? ['setting up'] : check.missing)}' : trouble?.message;
    final here = each == session.source && !_stars;
    return _railTile(
      context,
      key: Key('source ${each.label}'),
      leading: SourceMark.of(each, size: 18),
      title: each.displayName,
      caption: needs,
      selected: here,
      enabled: draft || each == session.source,
      tooltip: draft || each == session.source ? needs ?? each.displayName : 'A different agent needs a new conversation',
      captionColor: look.danger,
      onTap: () => _pickSource(each),
    );
  }

  Widget _railTile(
    BuildContext context, {
    required Key key,
    required Widget leading,
    required String title,
    String? caption,
    Color? captionColor,
    required bool selected,
    bool enabled = true,
    String? tooltip,
    required VoidCallback onTap,
  }) {
    final look = context.chatLook;
    final tile = InkWell(
      key: key,
      borderRadius: BorderRadius.circular(8),
      onTap: enabled ? onTap : null,
      child: Container(
        constraints: const BoxConstraints(minHeight: 40),
        margin: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        decoration: BoxDecoration(
          color: selected ? look.raised : null,
          borderRadius: BorderRadius.circular(8),
          border: Border(left: BorderSide(color: selected ? look.accent : Colors.transparent, width: 3)),
        ),
        child: Row(mainAxisSize: MainAxisSize.min, children: [
          IconTheme(data: IconThemeData(color: enabled ? null : look.muted), child: leading),
          const SizedBox(width: 8),
          Flexible(
            child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.start, children: [
              Text(title, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(fontSize: 13, color: enabled ? null : look.muted)),
              if (caption != null) Text(caption, maxLines: 2, overflow: TextOverflow.ellipsis, style: TextStyle(fontSize: 11, color: captionColor ?? look.muted)),
            ]),
          ),
        ]),
      ),
    );
    return tooltip == null ? tile : Tooltip(message: tooltip, child: tile);
  }

  // ---- the account, the search and the models

  Widget _middle(BuildContext context, List<ConfigOption> options, List<_ModelRow> rows) {
    final look = context.chatLook;
    final source = session.source;
    final trouble = source == null ? null : sourceTrouble(source, store.choicesOf(source), accounts: kit.accounts);
    final modelOption = optionIn(options, OptionRole.model);
    final known = modelOption != null || pendingChoice(session, OptionRole.model) != null;
    final highlighted = rows.isEmpty ? null : rows[_highlight.clamp(0, rows.length - 1)];
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      _accounts(context),
      TextField(
        key: const Key('picker-search'),
        controller: _search,
        focusNode: _focus,
        autofocus: true,
        decoration: InputDecoration(
          hintText: 'Search models…',
          prefixIcon: const Icon(Icons.search, size: 18),
          isDense: true,
          border: UnderlineInputBorder(borderSide: BorderSide(color: look.border)),
        ),
        onSubmitted: (_) {
          if (highlighted == null) return;
          _pickModel(highlighted);
          _close();
        },
      ),
      const SizedBox(height: 4),
      Expanded(
        child: trouble != null && !known && !_stars
            ? _troubleNote(context, trouble)
            : !known && !_stars
                ? Padding(
                    padding: const EdgeInsets.all(12),
                    child: Text('Its models are listed once the agent has started.', key: const Key('picker-after-start'), style: TextStyle(color: look.muted)),
                  )
                : rows.isEmpty
                    ? Padding(
                        padding: const EdgeInsets.all(12),
                        child: Text(
                          _stars && _search.text.isEmpty ? 'No favourites yet. Star a model to keep it here.' : 'No model matches.',
                          key: const Key('picker-no-models'),
                          style: TextStyle(color: look.muted),
                        ),
                      )
                    : ListView(children: [for (var i = 0; i < rows.length; i++) _modelRow(context, rows[i], i == _highlight)]),
      ),
    ]);
  }

  Widget _troubleNote(BuildContext context, SourceTrouble trouble) {
    final look = context.chatLook;
    final act = canAct(trouble, kit) ? () => unawaited(actOnTrouble(context, store, kit, trouble)) : null;
    return Padding(
      padding: const EdgeInsets.all(12),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Row(children: [
          Icon(Icons.error_outline, size: 16, color: look.danger),
          const SizedBox(width: 6),
          Flexible(child: Text(trouble.message, key: const Key('picker-trouble'), style: TextStyle(color: look.danger))),
        ]),
        if (trouble.detail != null) Padding(padding: const EdgeInsets.only(top: 4), child: Text(trouble.detail!, style: TextStyle(fontSize: 12, color: look.muted))),
        if (act != null) TextButton(key: const Key('picker-trouble-action'), onPressed: act, child: Text(trouble.kind.action)),
      ]),
    );
  }

  Widget _modelRow(BuildContext context, _ModelRow row, bool highlighted) {
    final look = context.chatLook;
    final model = pendingChoice(session, OptionRole.model)?.value ?? (draft ? null : optionIn(store.optionsOf(session), OptionRole.model)?.current);
    final inSource = row.source == null || row.source == session.source;
    final current = inSource && row.value == (model == '' ? null : model) && (row.value != null || model == null);
    final enabled = draft || row.source == null || row.source == session.source;
    final ledger = store.ledger;
    final owner = (row.source ?? session.source)?.label;
    final starred = row.value != null && owner != null && (ledger?.isFavourite(owner, row.value!) ?? false);
    return InkWell(
      key: ValueKey('model-choice=${row.value ?? ''}'),
      borderRadius: BorderRadius.circular(8),
      onTap: enabled ? () => _pickModel(row) : null,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        decoration: BoxDecoration(color: highlighted ? look.raised : null, borderRadius: BorderRadius.circular(8)),
        child: Row(children: [
          Expanded(
            child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
              Text(row.label, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(color: enabled ? null : look.muted)),
              if (row.detail != null) Text(row.detail!, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(fontSize: 11.5, color: look.muted)),
            ]),
          ),
          if (current) const Padding(padding: EdgeInsets.only(left: 6), child: Icon(Icons.check, size: 16)),
          if (row.value != null && ledger != null)
            IconButton(
              key: Key('star ${row.value}'),
              tooltip: starred ? 'Take it out of the favourites' : 'Add to the favourites',
              visualDensity: VisualDensity.compact,
              iconSize: 18,
              icon: Icon(starred ? Icons.star : Icons.star_border, color: starred ? look.accent : look.muted),
              onPressed: owner == null ? null : () => ledger.toggleFavourite(owner, row.value!),
            ),
          if (!widget.phone && row.number != null)
            Container(
              key: Key('shortcut ${row.number}'),
              margin: const EdgeInsets.only(left: 4),
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
              decoration: BoxDecoration(color: look.surface, borderRadius: BorderRadius.circular(6), border: Border.all(color: look.border)),
              child: Text('Ctrl+${row.number}', style: TextStyle(fontSize: 11, color: look.muted)),
            ),
        ]),
      ),
    );
  }

  Widget _accounts(BuildContext context) {
    final accounts = kit.accounts;
    final entry = session.source?.hostEntry ?? session.agent;
    if (accounts == null) return const SizedBox.shrink();
    final offered = accounts.offeredBy(entry);
    if (offered.isEmpty) return const SizedBox.shrink();
    final look = context.chatLook;
    final chosen = offered.where((each) => each.name == session.account).firstOrNull ?? offered.first;
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text('Account', style: TextStyle(fontSize: 11.5, color: look.muted)),
        const SizedBox(height: 2),
        Wrap(spacing: 6, runSpacing: 6, children: [
          for (final account in offered) _accountTile(context, accounts, account, chosen: account.name == chosen.name, now: now),
          if (draft && kit.onAccountSettings != null)
            ActionChip(
              key: const Key('account-add'),
              avatar: const Icon(Icons.add, size: 16),
              label: const Text('Add account…'),
              onPressed: () {
                _close();
                kit.onAccountSettings!();
              },
            ),
        ]),
      ]),
    );
  }

  Widget _accountTile(BuildContext context, AccountsController accounts, AccountInfo account, {required bool chosen, required int now}) {
    final look = context.chatLook;
    final holder = accounts.heldBy(account.name);
    final usable = account.usable && holder == null;
    final enabled = draft && usable;
    final state = holder == null ? account.statusLabel : 'In use by "$holder"';
    final summary = windowSummary(accounts.usageOf(account), now);
    return Tooltip(
      message: draft ? state : 'Another account needs a new conversation',
      child: InkWell(
        key: Key('account-choice ${account.name}'),
        borderRadius: BorderRadius.circular(8),
        onTap: enabled ? () => store.setAccount(session, account.name) : null,
        child: Container(
          constraints: const BoxConstraints(minWidth: 120),
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(8),
            color: chosen ? look.raised : null,
            border: Border.all(color: chosen ? look.accent : look.border),
          ),
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Row(mainAxisSize: MainAxisSize.min, children: [
              Icon(chosen ? Icons.check : Icons.account_circle_outlined, size: 14, color: usable ? null : look.muted),
              const SizedBox(width: 4),
              Text(account.name, style: TextStyle(fontSize: 13, color: usable || !draft ? null : look.muted)),
            ]),
            Text(state, style: TextStyle(fontSize: 11, color: usable ? look.muted : look.danger)),
            if (summary != null) Text(summary, key: Key('account-left-summary ${account.name}'), style: TextStyle(fontSize: 11, color: look.muted)),
          ]),
        ),
      ),
    );
  }

  // ---- the effort, the mode

  Widget _effort(BuildContext context, List<ConfigOption> options, {required bool wrap}) {
    final look = context.chatLook;
    final option = optionIn(options, OptionRole.effort);
    final pending = pendingChoice(session, OptionRole.effort);
    if (option == null && pending == null) {
      return wrap ? const SizedBox.shrink(key: Key('option-effort')) : SizedBox(key: const Key('option-effort'), child: Text('No effort to choose for this model.', style: TextStyle(fontSize: 12, color: look.muted)));
    }
    final current = pending?.value ?? (draft ? null : option?.current);
    Widget item(String? value, String label) {
      final selected = (value ?? '') == (current ?? '');
      void choose() {
        chooseRole(store, session, OptionRole.effort, option, value);
        _close();
      }
      final key = ValueKey('effort-choice=${value ?? ''}');
      if (wrap) return ChoiceChip(key: key, label: Text(label), selected: selected, onSelected: (_) => choose());
      return InkWell(
        key: key,
        borderRadius: BorderRadius.circular(8),
        onTap: choose,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 8),
          child: Row(children: [Expanded(child: Text(label)), if (selected) const Icon(Icons.check, size: 16)]),
        ),
      );
    }
    final items = [
      if (draft) item(null, option?.defaultLabel ?? 'Default'),
      for (final choice in option?.choices ?? const <Choice>[]) item(choice.value, choiceLabel(OptionRole.effort, choice)),
    ];
    return Column(key: const Key('option-effort'), mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Padding(padding: const EdgeInsets.only(bottom: 4), child: Text('Effort', style: TextStyle(fontSize: 11.5, color: look.muted))),
      if (wrap) Wrap(spacing: 6, runSpacing: 4, children: items) else Flexible(child: ListView(shrinkWrap: true, children: items)),
    ]);
  }

  Widget _bottom(BuildContext context, List<ConfigOption> options) {
    final look = context.chatLook;
    final option = modeOptionIn(options);
    final entries = modeEntries(option);
    final current = store.modeOf(session);
    final enabled = store.canChangeMode(session) || draft;
    final note = draft
        ? null
        : 'A different agent or account needs a new conversation. A model or effort chosen here applies from the next turn.';
    return Column(crossAxisAlignment: CrossAxisAlignment.start, mainAxisSize: MainAxisSize.min, children: [
      if (session.status != SessionStatus.preview)
        Wrap(spacing: 6, runSpacing: 4, crossAxisAlignment: WrapCrossAlignment.center, children: [
          Text('Mode', style: TextStyle(fontSize: 11.5, color: look.muted)),
          if (draft)
            ChoiceChip(
              key: const Key('picker-mode-default'),
              label: const Text('Default'),
              selected: current == null,
              onSelected: enabled ? (_) => store.clearMode(session) : null,
            ),
          for (final entry in entries)
            Tooltip(
              message: entry.description ?? entry.label,
              child: ChoiceChip(
                key: Key('picker-mode ${entry.value}'),
                label: Text(entry.label),
                selected: entry.value == current,
                onSelected: enabled && entry.selectable
                    ? (_) => unawaited(entry.own && session.bypass ? store.leaveBypass(session) : store.chooseMode(session, entry))
                    : null,
              ),
            ),
        ]),
      if (note != null) Padding(padding: const EdgeInsets.only(top: 6), child: Text(note, key: const Key('picker-note'), style: TextStyle(fontSize: 11.5, color: look.muted))),
    ]);
  }

  static String _and(List<String> words) =>
      words.length < 2 ? words.join() : '${words.sublist(0, words.length - 1).join(', ')} and ${words.last}';
}
