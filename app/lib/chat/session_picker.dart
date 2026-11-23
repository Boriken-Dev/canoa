import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart';
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';

import '../accounts/account_models.dart';
import '../accounts/accounts_controller.dart';
import '../accounts/add_account_dialog.dart';
import '../accounts/usage_view.dart' show windowSummary;
import '../history/agent_switch.dart' show cannotTake;
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

/// What the login the agent has where it runs is called.
const machineLoginLabel = "This machine's login";

/// The account for [entry]'s session named [account], or null when it runs on the login the agent has where it runs: the person's
/// own choice of that, else the entry's first listed account, else that login too.
AccountInfo? chosenAccount(AccountsController accounts, String entry, String? account) {
  if (account == machineLogin) return null;
  // A pool is a choice of its own, not an account: the pool tile says so, and no account tile is the chosen one.
  if (chosenPool(accounts, entry, account) != null) return null;
  final named = accounts.availableTo(entry).where((each) => each.name == account).firstOrNull;
  return named ?? accounts.offeredBy(entry).firstOrNull;
}

/// The pool [entry]'s session runs on when [account] names one, or none was named and the entry's first is a pool; else null.
EntryPool? chosenPool(AccountsController accounts, String entry, String? account) {
  if (account == machineLogin) return null;
  return accounts.poolOf(entry, account ?? accounts.defaultOf(entry));
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
    if (!draft) unawaited(kit.portable?.refresh());
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
        // The models of a pool say which of its accounts list each.
        final detail = [
          ?listed?.description,
          if (listed != null && listed.accounts.isNotEmpty) 'On ${listed.accounts.join(', ')}',
        ].join(' · ');
        rows.add(_ModelRow(value: choice.value, label: choiceLabel(OptionRole.model, choice), detail: detail.isEmpty ? null : detail));
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
    if (source == session.source) return;
    if (!draft) {
      // A running conversation goes with the person to another agent: the host exports it and imports it there.
      final switchAgent = kit.switchAgent;
      if (switchAgent == null) return;
      _close();
      unawaited(switchAgent(session, source));
      return;
    }
    store.updateDraft(session, source: source);
    setState(() => _stars = false);
  }

  /// Why a running conversation cannot go to [source]; null when it can, and always for a draft or a source it is on.
  String? _cannotSwitchTo(AgentSource source) {
    if (draft || source == session.source) return null;
    final switchAgent = kit.switchAgent, portable = kit.portable;
    if (switchAgent == null || portable == null) return 'A different agent needs a new conversation';
    if (!portable.asked) return 'Asking the host which agents can take a conversation…';
    if (portable.failed) return 'The host did not say which agents can take a conversation.';
    return cannotTake(portable.of(source.label), name: source.displayName) ?? store.agentSwitchBlock(session);
  }

  void _pickNumber(int n, List<_ModelRow> rows) {
    final row = rows.where((each) => each.number == n).firstOrNull;
    if (row == null) return;
    _pickModel(row);
    _close();
  }

  @override
  Widget build(BuildContext context) {
    final listenables = <Listenable>[store, ?store.ledger, ?kit.accountsFor(session), ?kit.portable];
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
      for (final each in kit.sourcesFor(session)) _sourceTile(context, each),
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
    final trouble = sourceTrouble(each, choices, accounts: kit.accountsFor(session));
    final check = kit.checks[each];
    final needs = check != null && !check.ready ? 'Needs ${_and(check.missing.isEmpty ? ['setting up'] : check.missing)}' : trouble?.message;
    final here = each == session.source && !_stars;
    final cannot = _cannotSwitchTo(each);
    final portable = kit.portable?.of(each.label);
    // A source that cannot take a conversation says so under its name; a wait or a busy agent is said in the tooltip alone.
    final saysNo = !draft && each != session.source && kit.portable?.asked == true && kit.portable?.failed == false && cannotTake(portable, name: each.displayName) != null;
    return _railTile(
      context,
      key: Key('source ${each.label}'),
      leading: SourceMark.of(each, size: 18),
      title: each.displayName,
      caption: saysNo ? 'Cannot take a conversation' : needs,
      selected: here,
      enabled: (draft || each == session.source || cannot == null) && (draft || each == session.source || needs == null),
      tooltip: cannot ?? needs ?? each.displayName,
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
    final trouble = source == null ? null : sourceTrouble(source, store.choicesOf(source), accounts: kit.accountsFor(session));
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
                    child: Text(
                      draft ? 'Its models are listed once the agent has started.' : 'This agent offers no model choice in a running conversation.',
                      key: const Key('picker-after-start'),
                      style: TextStyle(color: look.muted),
                    ),
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
    final act = canAct(trouble, kit, session: session) ? () => unawaited(actOnTrouble(context, store, kit, trouble, session: session)) : null;
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
    final accounts = kit.accountsFor(session);
    final entry = session.source?.hostEntry ?? session.agent;
    if (accounts == null) return const SizedBox.shrink();
    final harness = accounts.harnessOf(entry);
    if (harness == null) return const SizedBox.shrink();
    final offered = accounts.availableTo(entry);
    final look = context.chatLook;
    final chosen = chosenAccount(accounts, entry, session.account);
    final pool = chosenPool(accounts, entry, session.account);
    final pools = accounts.poolsOf(entry);
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        Text('Account', style: TextStyle(fontSize: 11.5, color: look.muted)),
        const SizedBox(height: 2),
        ConstrainedBox(constraints: const BoxConstraints(maxHeight: 260), child: SingleChildScrollView(child: SizedBox(width: double.infinity, child: Wrap(spacing: 6, runSpacing: 6, children: [
          _accountTile(context, accounts, null, chosen: chosen == null && pool == null, now: now),
          for (final account in offered) _accountTile(context, accounts, account, chosen: account.name == chosen?.name, now: now),
          for (final each in pools) _poolTile(context, each, chosen: each.name == pool?.name),
          ActionChip(
            key: const Key('account-add'),
            avatar: const Icon(Icons.add, size: 16),
            label: const Text('Add account…'),
            onPressed: () async {
              await showDialog<AccountInfo>(context: context, builder: (_) => AddAccountDialog(accounts: accounts, provider: accounts.providers.where((each) => each.agent == harness).firstOrNull?.id));
              unawaited(accounts.refresh());
            },
          ),
        ])))),
        if (accounts.acceptsOf(entry) case final accepts when accepts.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text('$entry accepts ${standardsText(accepts)}', key: const Key('picker-accepts'), style: TextStyle(fontSize: 11.5, color: look.muted)),
          ),
        if (_accountBlock(accounts) case final block? when offered.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(block, key: const Key('picker-account-note'), style: TextStyle(fontSize: 11.5, color: look.muted)),
          ),
      ]),
    );
  }

  /// One pool the entry offers: accounts that provide one standard, which a session runs on whichever is free and lists the model.
  /// It is chosen for a draft; a conversation that has started keeps the account it runs on, which is said here.
  Widget _poolTile(BuildContext context, EntryPool pool, {required bool chosen}) {
    final look = context.chatLook;
    final enabled = draft && pool.members.isNotEmpty;
    final members = pool.members.map((each) => each.name == localAccount ? machineLoginLabel : each.name).join(', ');
    final ran = !draft && chosen ? session.ranOnAccount : null;
    final state = [
      standardTitle(pool.standard),
      if (pool.automatic) 'made by Canoa',
      if (ran != null) 'running on $ran' else if (pool.members.isEmpty) 'no account yet' else members,
    ].join(' · ');
    return Tooltip(
      message: draft ? 'A session on a pool runs on whichever of its accounts is free' : 'A pool is chosen for a new conversation',
      child: InkWell(
        key: Key('pool-choice ${pool.name}'),
        borderRadius: BorderRadius.circular(8),
        onTap: enabled ? () => store.setAccount(session, pool.name) : null,
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
              Icon(Icons.layers_outlined, size: 16, color: enabled || chosen ? null : look.muted, key: Key('pool-mark ${pool.name}')),
              const SizedBox(width: 6),
              Text(pool.name, style: TextStyle(fontSize: 13, color: enabled || chosen ? null : look.muted)),
              if (chosen) ...[const SizedBox(width: 4), const Icon(Icons.check, size: 14)],
            ]),
            Text(state, key: Key('pool-state ${pool.name}'), style: TextStyle(fontSize: 11, color: look.muted)),
          ]),
        ),
      ),
    );
  }

  /// Has this agent offer [account], the person's own choice written to canoa.cfg, then chooses it. An agent open to every paired
  /// key says so first, as Settings does.
  Future<void> _offerHere(BuildContext context, AccountsController accounts, AccountInfo account) async {
    final entry = session.source?.hostEntry ?? session.agent;
    if (accounts.isOpen(entry)) {
      final sure = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          key: const Key('offer-here-open'),
          title: Text('Offer ${account.name} to $entry?'),
          content: Text(
            '$entry has no list of keys, so every key paired with this host can start a session on it, running the agent as the user '
            'who runs this host, and a session there may run on ${account.name}. Add the keys that may use it to its peers in canoa.cfg, '
            'or keep ${account.name} to an agent only this computer reaches.',
          ),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Do not offer it')),
            FilledButton(key: const Key('offer-here-confirm'), onPressed: () => Navigator.pop(context, true), child: const Text('Offer it to every paired key')),
          ],
        ),
      );
      if (sure != true) return;
    }
    await accounts.setOffered(account, entry, offered: true);
    store.setAccount(session, account.name);
  }

  /// Why a running session cannot go on under another account now; null when it can, or when it has not started.
  String? _accountBlock(AccountsController accounts) {
    if (draft) return null;
    return store.accountSwitchBlock(session);
  }

  /// One account's tile, or, for a null [account], the login the agent has where it runs, which every agent that takes accounts can
  /// start on.
  Widget _accountTile(BuildContext context, AccountsController accounts, AccountInfo? account, {required bool chosen, required int now}) {
    final look = context.chatLook;
    final name = account?.name ?? machineLoginLabel;
    final holder = account == null ? null : accounts.heldBy(account.name);
    final usable = (account?.usable ?? true) && holder == null;
    final block = _accountBlock(accounts);
    final canMove = !draft && !chosen && block == null && kit.switchAccount != null;
    final entry = session.source?.hostEntry ?? session.agent;
    final offerFirst = account != null && accounts.needsOffer(entry, account);
    final enabled = usable && !offerFirst && (draft || canMove);
    final state = !draft && chosen
        ? 'This conversation'
        : account == null
            ? 'The login on this computer'
            : offerFirst
                ? 'Its key reaches the agent; offer it to this agent first'
                : holder == null ? account.statusLabel : 'In use by "$holder"';
    final summary = account == null ? null : windowSummary(accounts.usageOf(account), now);
    return Tooltip(
      message: draft || chosen || block == null ? state : block,
      child: InkWell(
        key: account == null ? const Key('account-machine') : Key('account-choice ${account.name}'),
        borderRadius: BorderRadius.circular(8),
        onTap: !enabled
            ? null
            : draft
                ? () => store.setAccount(session, account?.name ?? machineLogin)
                : () {
                    _close();
                    unawaited(kit.switchAccount!(session, account?.name ?? machineLogin));
                  },
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
              Opacity(
                opacity: usable ? 1 : 0.5,
                child: SourceMark.provider(
                  account == null ? accounts.harnessOf(entry) : accountMarkId(account.groupId, account.kind),
                  size: 16,
                  key: Key('account-mark $name'),
                ),
              ),
              const SizedBox(width: 6),
              Text(name, style: TextStyle(fontSize: 13, color: usable || !draft ? null : look.muted)),
              if (chosen) ...[const SizedBox(width: 4), const Icon(Icons.check, size: 14)],
            ]),
            Text(account != null && account.types.isNotEmpty ? '${standardsText(account.types)} · $state' : state, key: Key('account-state $name'), style: TextStyle(fontSize: 11, color: usable ? look.muted : look.danger)),
            if (summary != null) Text(summary, key: Key('account-left-summary $name'), style: TextStyle(fontSize: 11, color: look.muted)),
            if (offerFirst && draft)
              TextButton(
                key: Key('account-offer $name'),
                onPressed: () => unawaited(_offerHere(context, accounts, account)),
                child: const Text('Offer to this agent'),
              ),
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
      return wrap ? const SizedBox.shrink(key: Key('option-effort')) : SizedBox(key: const Key('option-effort'), child: Text(draft ? 'No effort to choose for this model.' : 'This agent offers no effort choice in a running conversation.', style: TextStyle(fontSize: 12, color: look.muted)));
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
        : '${kit.switchAgent == null || kit.portable?.failed == true ? 'A different agent needs a new conversation.' : 'Another agent takes the whole conversation with it: it is exported from this one and imported there.'}${session.status == SessionStatus.working ? ' A model or effort chosen now applies from the next turn.' : ''}';
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
              message: modeDescription(option, entry) ?? entry.label,
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
