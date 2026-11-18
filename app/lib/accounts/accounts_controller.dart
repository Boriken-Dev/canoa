import 'dart:async';

import 'package:coqui_acp_chat/coqui_acp_chat.dart' show UsageReport;
import 'package:flutter/foundation.dart';

import '../config/canoa_config.dart';
import 'account_models.dart';
import 'accounts_host.dart';

/// Takes the configuration after an account was made or removed, writes it and has the host serve it: the app's own writer.
typedef ChangeAccountConfig = Future<void> Function(CanoaConfig Function(CanoaConfig config) edit);

/// The accounts Settings lists and the composer offers: what the host says of each, the kinds of each provider, which entries
/// offer which, and what a session holds.
class AccountsController extends ChangeNotifier {
  AccountsController({
    required this.host,
    required this.config,
    required this.change,
    this.harnesses = const ['claude', 'codex', 'opencode'],
    this.usage,
    this.holder,
    this.hostIsHere = true,
  });

  final AccountsHost host;

  /// The configuration as the app holds it now: which entries offer which accounts is read from it.
  final CanoaConfig Function() config;

  /// Writes a change to canoa.cfg (an entry offering an account) and restarts the host so it serves it.
  final ChangeAccountConfig change;

  /// The providers an account can be added to, besides any a source of the host runs.
  final List<String> harnesses;

  /// What the sessions on an account have last heard of its allowances, by the account's id.
  final UsageReport? Function(String accountId)? usage;

  /// Whether the host runs on the machine this app does. It does not for the web page a host serves and for a host elsewhere: a browser
  /// sign-in that comes back to the host's loopback then reaches nothing, and a device code is chosen first.
  final bool hostIsHere;

  /// The title of the open session that runs on [account], or null when none does.
  final String? Function(String account)? holder;


  List<AccountInfo> _accounts = const [];
  List<AccountEntry> _entries = const [];
  final _catalogs = <String, AccountCatalog>{};
  String? _problem;
  bool _loading = false;
  bool _disposed = false;

  List<AccountInfo> get accounts => _accounts;

  /// The agent entries the host lists, which a built-in source is one of before canoa.cfg names it.
  List<AccountEntry> get hostEntries => _entries;

  /// canoa.cfg as the host serves it: the entries the host lists, with what each offers, over the app's own configuration.
  CanoaConfig get _view => config().withHostEntries(_entries);
  String? get problem => _problem;
  bool get loading => _loading;

  /// Every provider an account can be added to: the host's, then the sources' that run one.
  List<String> get providers => {...harnesses, ..._accounts.map((each) => each.harness)}.toList();

  /// The accounts of [harness], in canoa.cfg's order.
  List<AccountInfo> of(String harness) => [for (final each in _accounts) if (each.harness == harness) each];

  AccountInfo? named(String name) => _accounts.where((each) => each.name == name).firstOrNull;

  /// The accounts the agent entry [entry] offers a client, in the order it lists them, the first being the default. Empty when it
  /// offers none: its harness then runs on the login it has where it runs.
  List<AccountInfo> offeredBy(String entry) => [
    for (final name in _view.hostAccountsOf(entry)) ?named(name),
  ];

  /// The provider the agent entry [entry] runs, which takes accounts; null for an entry that runs a program of its own.
  String? harnessOf(String entry) {
    final harness = _view.hostHarnessOf(entry);
    return harness != null && providers.contains(harness) ? harness : null;
  }

  /// What the host lists of [entry], or null for one it does not list.
  AccountEntry? hostEntry(String entry) => _entries.where((each) => each.name == entry).firstOrNull;

  /// The credential standards [entry]'s agent accepts, the most preferred first; empty when the host does not say.
  List<String> acceptsOf(String entry) => hostEntry(entry)?.accepts ?? const [];

  /// Every account a session of [entry] can run on: those the entry lists, in its order, then the others. Which others is the host's
  /// match of what each account supports to what the agent accepts, whichever provider made the account; a host that does not say
  /// leaves the entry's provider's own. A key the host trusts with accounts may choose any of them, so the choice is not narrowed to
  /// what canoa.cfg lists.
  List<AccountInfo> availableTo(String entry) {
    final harness = harnessOf(entry);
    if (harness == null) return const [];
    final listed = offeredBy(entry);
    final usable = hostEntry(entry)?.usable;
    final rest = usable == null ? of(harness) : [for (final each in _accounts) if (usable.contains(each.name)) each];
    return [...listed, for (final each in rest) if (!listed.any((one) => one.name == each.name)) each];
  }

  /// Whether the credential of [account] reaches the agent in its environment (an API key or a token, not a login kept in a folder
  /// of its own), so a repository's settings could redirect it unless the entry that runs it lists it and limits them.
  bool keyReachesAgent(AccountInfo account) {
    final kind = catalogIfKnown(account.harness)?.kind(account.kind);
    return kind != null && !kind.stateDir && kind.endpoints.any((each) => each.purpose == 'model_api' && each.key != null);
  }

  /// Whether a session of [entry] cannot start on [account] until the entry offers it: its key reaches the agent.
  bool needsOffer(String entry, AccountInfo account) => !offeredBy(entry).any((each) => each.name == account.name) && keyReachesAgent(account);

  /// The agent entries that run the provider [harness], which can offer its accounts.
  List<String> entriesRunning(String harness) => _view.hostEntriesRunning(harness);

  /// Whether the entry [entry] is open to every paired key, so an account offered there reaches them all.
  bool isOpen(String entry) => _view.hostEntryIsOpen(entry);

  /// The entries that offer [account].
  List<String> offering(String account) => [
    for (final entry in _view.hostEntries)
      if (_view.hostAccountsOf(entry.name).contains(account)) entry.name,
  ];

  /// What the host last said of each kind and sign-in method of [harness], asked once.
  Future<AccountCatalog> catalog(String harness) async => _catalogs[harness] ??= await host.catalog(harness);

  AccountCatalog? catalogIfKnown(String harness) => _catalogs[harness];

  /// The title of the session holding [account], if one does.
  String? heldBy(String account) => holder?.call(account);

  /// The usage the sessions on [account] last heard of its allowances.
  UsageReport? usageOf(AccountInfo account) => usage?.call(account.id);

  /// Asks the host which accounts there are, and for the catalog of each provider they use.
  Future<void> refresh() async {
    // One in flight may have asked before what changed: ask once more when it is over, so the last word is the newest.
    if (_loading) {
      _again = true;
      return;
    }
    _loading = true;
    _notify();
    try {
      _accounts = await host.list();
      try {
        _entries = await host.entries();
      } on Object {
        // A host with no list of its entries leaves what canoa.cfg names.
      }
      _problem = null;
      for (final harness in {..._accounts.map((each) => each.harness)}) {
        try {
          await catalog(harness);
        } on Object {
          // The kind of one provider is unknown: its accounts are listed all the same.
        }
      }
    } on Object catch (error) {
      _problem = 'The host did not list the accounts: $error';
    } finally {
      _loading = false;
      _notify();
    }
    if (_again && !_disposed) {
      _again = false;
      await refresh();
    }
  }

  bool _again = false;

  /// Signs in and makes [name]; the account is offered by an entry running [harness] once it is made.
  SignIn add({required String name, required String harness, required String method, Map<String, String> inputs = const {}}) {
    final run = host.signIn(name: name, harness: harness, method: method, inputs: inputs);
    void ended() {
      if (run.phase != SignInPhase.done) return;
      run.removeListener(ended);
      unawaited(_made(name, harness));
    }

    run.addListener(ended);
    return run;
  }

  Future<void> _made(String name, String harness) async {
    try {
      await change((config) => config.withAccountOffered(account: name, harness: harness));
    } on Object catch (error) {
      _problem = 'Canoa could not offer $name to a source: $error';
    }
    await refresh();
  }

  /// Signs in again to [account], which needs it or is being replaced.
  SignIn signInAgain(AccountInfo account, {String? method}) {
    final run = host.signIn(name: account.name, harness: account.harness, method: method, again: true);
    void ended() {
      if (run.phase != SignInPhase.done) return;
      run.removeListener(ended);
      unawaited(refresh());
    }

    run.addListener(ended);
    return run;
  }

  /// Offers [account] to the entry [entry], which runs its provider, or stops offering it there.
  Future<void> setOffered(AccountInfo account, String entry, {required bool offered}) async {
    await change(
      (config) => offered
          ? config.withAccountOffered(account: account.name, harness: account.harness, entry: entry)
          : config.withoutAccountOffered(account.name, only: entry),
    );
    _notify();
  }

  /// Forgets [account]: no entry offers it any more, then the host takes its credential and place away. A refusal (a session holds
  /// it) is the host's words, thrown as they are, and the offers stay as they were.
  Future<void> remove(AccountInfo account) async {
    // What each entry offered, in its order, so a refusal puts back exactly that and not the account in another place.
    final before = {for (final entry in offering(account.name)) entry: _view.hostAccountsOf(entry)};
    await change((config) => config.withoutAccountOffered(account.name));
    try {
      await host.remove(account.name);
    } on Object {
      await change((config) {
        var restored = config;
        for (final MapEntry(:key, :value) in before.entries) {
          restored = restored.withAccountsOf(key, value);
        }
        return restored;
      });
      rethrow;
    } finally {
      await refresh();
    }
  }

  /// The configuration changed under it: what entries offer is read again.
  void changed() => _notify();

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
