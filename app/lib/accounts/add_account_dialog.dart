import 'dart:async';

import 'package:coqui_acp_chat_ui/coqui_acp_chat_ui.dart' show SecretRequestCard;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../providers/provider_catalog.dart';
import '../sources/source_icons.dart';
import '../store/links.dart';
import '../store/programs.dart' show ProgramAsk;
import '../store/programs_view.dart' show AskDialog;
import 'account_models.dart';
import 'accounts_controller.dart';
import 'accounts_host.dart';

/// The rule for an account's name, which the host holds to as it does an entry's.
final RegExp accountName = RegExp(r'^[A-Za-z0-9_-]{1,64}$');

/// What the person sees of a provider's name: `claude` is Claude.
String harnessTitle(String harness) => providerCatalog.titleOf(harness);

/// Adds an account: the provider and a name, then one of the provider's own ways to sign in, then what that way asks. A key or
/// a code is typed into the sealed form the host asks for, which goes to the host and is never held here.
class AddAccountDialog extends StatefulWidget {
  const AddAccountDialog({super.key, required this.accounts, this.harness});

  final AccountsController accounts;

  /// The provider to add to; the first the host has when null.
  final String? harness;

  @override
  State<AddAccountDialog> createState() => _AddAccountDialogState();
}

class _AddAccountDialogState extends State<AddAccountDialog> {
  late String _harness = widget.harness ?? widget.accounts.providers.firstOrNull ?? '';
  final _name = TextEditingController();
  Future<AccountCatalog>? _catalog;
  String? _method;
  String? _problem;
  SignIn? _run;

  @override
  void initState() {
    super.initState();
    _name.text = _suggest();
    _load();
  }

  String _suggest() {
    var name = _harness.toLowerCase(), n = 2;
    while (widget.accounts.named(name) != null) {
      name = '${_harness.toLowerCase()}-${n++}';
    }
    return name;
  }

  void _load() {
    _method = null;
    final catalog = _catalog = widget.accounts.catalog(_harness);
    // Chosen before the person chooses, unless they have by the time the host answers or have moved to another provider.
    unawaited(catalog.then<void>((made) {
      if (!mounted || _catalog != catalog || _method != null) return;
      final first = made.preferred(hostIsHere: widget.accounts.hostIsHere);
      if (first != null) setState(() => _method = first.id);
    }, onError: (Object _) {}));
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  String? get _nameProblem {
    final name = _name.text;
    if (!accountName.hasMatch(name)) return 'Use 1 to 64 letters, digits, - or _';
    if (widget.accounts.named(name) != null) return 'An account is called that already';
    return null;
  }

  Future<void> _start(AccountCatalog catalog) async {
    final method = catalog.login(_method!);
    if (method == null) return;
    var inputs = <String, String>{};
    // A secret is asked in the sealed form once the sign-in runs, never in a form that runs before it: refuse rather than drop it.
    if (method.inputFields.values.any(ProgramAsk.sensitive)) {
      setState(() => _problem = '${method.title} asks for a secret before it signs in, which Canoa cannot take sealed here.');
      return;
    }
    setState(() => _problem = null);
    if (method.inputFields.isNotEmpty) {
      final given = await showDialog<({Map<String, Object?> values, Map<String, Object?> secrets})>(
        context: context,
        builder: (_) => AskDialog(question: ProgramAsk(method.title, method.inputs)),
      );
      if (given == null || !mounted) return;
      inputs = {for (final MapEntry(:key, :value) in given.values.entries) key: '$value'};
    }
    final run = widget.accounts.add(name: _name.text, harness: _harness, method: method.id, inputs: inputs);
    run.addListener(_changed);
    setState(() => _run = run);
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final run = _run;
    return AlertDialog(
      key: const Key('add-account'),
      title: const Text('Add account'),
      content: ConstrainedBox(constraints: const BoxConstraints(maxWidth: 460), child: SizedBox(width: double.maxFinite, child: run == null ? _choose(context) : SignInPanel(run: run))),
      actions: run == null ? _chooseActions() : _runActions(run),
    );
  }

  Widget _choose(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final providers = widget.accounts.providers;
    return SingleChildScrollView(
      child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        if (providers.length > 1)
          DropdownButtonFormField<String>(
            key: const Key('add-account-harness'),
            initialValue: _harness,
            decoration: const InputDecoration(labelText: 'Provider', border: OutlineInputBorder(), isDense: true),
            items: [for (final each in providers) DropdownMenuItem(value: each, child: Row(children: [SourceMark.provider(each, size: 18), const SizedBox(width: 8), Text(harnessTitle(each))]))],
            onChanged: (value) => setState(() {
              _harness = value ?? _harness;
              _name.text = _suggest();
              _load();
            }),
          ),
        if (providers.length > 1) const SizedBox(height: 12),
        TextField(
          key: const Key('add-account-name'),
          controller: _name,
          decoration: InputDecoration(
            labelText: 'Name',
            helperText: 'What the session picker calls it',
            errorText: _nameProblem,
            border: const OutlineInputBorder(),
            isDense: true,
          ),
          onChanged: (_) => setState(() {}),
        ),
        const SizedBox(height: 16),
        Text('Sign in with', style: Theme.of(context).textTheme.labelLarge),
        const SizedBox(height: 4),
        FutureBuilder<AccountCatalog>(
          future: _catalog,
          builder: (context, snapshot) {
            if (snapshot.connectionState != ConnectionState.done) {
              return const Padding(padding: EdgeInsets.all(12), child: Center(child: CircularProgressIndicator()));
            }
            final catalog = snapshot.data;
            if (snapshot.hasError || catalog == null) {
              return Text(
                'The host could not say how ${harnessTitle(_harness)} signs in: ${snapshot.error}',
                key: const Key('add-account-catalog-problem'),
                style: TextStyle(color: scheme.error),
              );
            }
            if (catalog.logins.isEmpty) return Text('${harnessTitle(_harness)} offers no way to sign in from here.');
            return RadioGroup<String>(
              groupValue: _method,
              onChanged: (value) => setState(() => _method = value),
              child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                for (final method in catalog.logins) _MethodTile(method: method, kind: catalog.kind(method.produces), harness: _harness),
              ]),
            );
          },
        ),
        if (_problem != null) Padding(padding: const EdgeInsets.only(top: 8), child: Text(_problem!, style: TextStyle(color: scheme.error))),
      ]),
    );
  }

  List<Widget> _chooseActions() => [
    TextButton(key: const Key('add-account-cancel'), onPressed: () => Navigator.pop(context), child: const Text('Cancel')),
    FutureBuilder<AccountCatalog>(
      future: _catalog,
      builder: (context, snapshot) => FilledButton(
        key: const Key('add-account-start'),
        onPressed: snapshot.data == null || _method == null || _nameProblem != null ? null : () => unawaited(_start(snapshot.data!)),
        child: const Text('Sign in'),
      ),
    ),
  ];

  List<Widget> _runActions(SignIn run) => [
    if (run.running)
      TextButton(
        key: const Key('add-account-cancel'),
        onPressed: () {
          run.cancel();
          Navigator.pop(context);
        },
        child: const Text('Cancel'),
      )
    else
      FilledButton(
        key: const Key('add-account-done'),
        onPressed: () => Navigator.pop(context, run.account),
        child: Text(run.phase == SignInPhase.done ? 'Done' : 'Close'),
      ),
  ];
}

class _MethodTile extends StatelessWidget {
  const _MethodTile({required this.method, required this.kind, required this.harness});

  final LoginMethodInfo method;
  final AccountKindInfo? kind;
  final String harness;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final small = Theme.of(context).textTheme.bodySmall?.copyWith(color: scheme.onSurfaceVariant);
    return RadioListTile<String>(
      key: Key('login-method ${method.id}'),
      contentPadding: EdgeInsets.zero,
      dense: true,
      value: method.id,
      secondary: SourceMark.provider(accountMarkId(harness, method.produces), size: 24, key: Key('method-mark ${method.id}')),
      title: Text(method.title),
      subtitle: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        if (method.description.isNotEmpty) Text(method.description, style: small),
        if (kind != null && kind!.provides.isNotEmpty) Text('Supports ${standardsText(kind!.provides)}', style: small),
        if (kind != null)
          for (final notice in kind!.notices) Text(notice, style: small?.copyWith(color: scheme.tertiary)),
      ]),
    );
  }
}

/// What a sign-in says and asks as it runs: its lines, a page to open with a code to confirm, the sealed form, and how it ended.
class SignInPanel extends StatelessWidget {
  const SignInPanel({super.key, required this.run});

  final SignIn run;

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: run,
    builder: (context, _) {
      final scheme = Theme.of(context).colorScheme;
      final page = run.page, asking = run.asking;
      return SingleChildScrollView(
        child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          for (final line in run.lines) Padding(padding: const EdgeInsets.only(bottom: 4), child: SelectableText(line)),
          if (page != null) _PageStep(page: page),
          if (asking != null)
            SecretRequestCard(
              key: ValueKey('sign-in-ask ${asking.request.context.requestId}'),
              request: asking.request,
              environment: asking.environment,
              agent: 'Canoa',
              onReply: run.answer,
            ),
          if (run.running && asking == null)
            const Padding(
              padding: EdgeInsets.only(top: 8),
              child: Wrap(crossAxisAlignment: WrapCrossAlignment.center, children: [
                SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2)),
                SizedBox(width: 12),
                Text('Waiting for the sign-in'),
              ]),
            ),
          if (run.phase == SignInPhase.done)
            SelectableText('Added ${run.name}. Canoa offers it to new sessions of ${harnessTitle(run.harness)}.', key: const Key('sign-in-done')),
          if (run.phase == SignInPhase.failed)
            SelectableText('It did not sign in: ${run.problem}', key: const Key('sign-in-failed'), style: TextStyle(color: scheme.error)),
        ]),
      );
    },
  );
}

class _PageStep extends StatelessWidget {
  const _PageStep({required this.page});

  final SignInPage page;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Container(
      key: const Key('sign-in-page'),
      margin: const EdgeInsets.symmetric(vertical: 8),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(color: scheme.surfaceContainerHighest, borderRadius: BorderRadius.circular(8)),
      child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
        const Text('Open this page to sign in'),
        const SizedBox(height: 4),
        Row(children: [
          Expanded(child: SelectableText(page.url, key: const Key('sign-in-url'))),
          IconButton(
            key: const Key('sign-in-copy-url'),
            tooltip: 'Copy the link',
            icon: const Icon(Icons.copy, size: 18),
            onPressed: () => Clipboard.setData(ClipboardData(text: page.url)),
          ),
        ]),
        if (webLink(page.url) == null)
          SelectableText('Canoa will not open this address: it is not a plain http or https link.', key: const Key('sign-in-url-refused'), style: TextStyle(color: scheme.error)),
        if (page.userCode case final code?) ...[
          const SizedBox(height: 8),
          const Text('and confirm this code there'),
          Row(children: [
            SelectableText(code, key: const Key('sign-in-code'), style: const TextStyle(fontFamily: 'monospace', fontSize: 18, letterSpacing: 2)),
            IconButton(
              key: const Key('sign-in-copy-code'),
              tooltip: 'Copy the code',
              icon: const Icon(Icons.copy, size: 18),
              onPressed: () => Clipboard.setData(ClipboardData(text: code)),
            ),
          ]),
        ],
        const SizedBox(height: 4),
        Wrap(spacing: 8, children: [
          OutlinedButton(
            key: const Key('sign-in-open'),
            onPressed: webLink(page.url) == null ? null : () => unawaited(openLink(webLink(page.url)!)),
            child: const Text('Open the page'),
          ),
        ]),
      ]),
    );
  }
}
