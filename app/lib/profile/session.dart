/// The base environment of a session: the variables every layer's `${...}` and every `from_env` sees before any layer is
/// applied. The same as `canoa_host::profile`'s `Session`.
///
/// Two kinds go in. The identity variables name what the session runs on, as dotagents stamps them (`AGENTS_HARNESS`,
/// `AGENTS_VENDOR`, `AGENT`, `AGENTS_MODEL`, `AGENTS_PROJECT_ROOT`) with Canoa's own beside them; the agent is given them
/// too, and a layer's own value for one replaces them. The host's own variables are those the host sets for the session
/// itself: a layer reads them and may set its own from them.
library;

import '../providers/provider_catalog.dart';

/// What identifies a session, as far as the host knows it. Each is set only when it has a value.
final class SessionWho {
  const SessionWho({
    this.harness,
    this.model,
    this.effort,
    this.version,
    this.account,
    this.accountKind,
    this.accountProvider,
    this.profile,
    this.loadout,
    this.role,
    this.project,
  });

  /// The AOS harness id (`claude-code`, `codex`, `pi`, ...).
  final String? harness;
  final String? model;
  final String? effort;

  /// The version of the program the Store installed for the harness.
  final String? version;

  /// The label of the account the session runs on, never anything of its credentials.
  final String? account;

  /// `subscription`, `api-key`, `gateway` or `local`.
  final String? accountKind;

  /// `anthropic`, `openai`, `xai`, `google`, or the vendor of what runs a local model.
  final String? accountProvider;
  final String? profile;
  final String? loadout;

  /// `delegate` when the session is a task another agent started.
  final String? role;

  /// The folder the session works in, where one is known.
  final String? project;
}

/// The variables a session starts from.
final class SessionVariables {
  /// Nothing: what a view with no session starts from.
  const SessionVariables.none() : identity = const {}, host = const {};

  /// The identity variables of [who] and the [host]'s own variables for the session. [own] reads this process's
  /// environment: a variable it already holds is kept, as dotagents keeps one.
  factory SessionVariables(
    SessionWho who, {
    Map<String, String> host = const {},
    String? Function(String name)? own,
  }) {
    final found = <String, String>{};
    void put(String name, String? value) {
      final held = own?.call(name);
      final chosen = held != null && held.isNotEmpty ? held : value;
      if (chosen != null && chosen.isNotEmpty) found[name] = chosen;
    }

    if (who.harness case final harness?) {
      final known = providerCatalog.byAos(harness);
      final variable = known?.modelVariable;
      put('AGENTS_HARNESS', harness);
      put('AGENTS_VENDOR', known?.vendor);
      put('AGENT', harness);
      put(
        'AGENTS_MODEL',
        who.model ?? (variable == null ? null : own?.call(variable)),
      );
    }
    put('AGENTS_HOST', 'canoa');
    put('AGENTS_HARNESS_VERSION', who.version);
    put('AGENTS_EFFORT', who.effort);
    put('AGENTS_ACCOUNT', who.account);
    put('AGENTS_ACCOUNT_KIND', who.accountKind);
    put('AGENTS_ACCOUNT_PROVIDER', who.accountProvider);
    put('AGENTS_PROFILE', who.profile);
    put('AGENTS_LOADOUT', who.loadout);
    put('AGENTS_ROLE', who.role);
    put('AGENTS_PROJECT_ROOT', who.project);
    return SessionVariables._(found, Map.unmodifiable(host));
  }

  const SessionVariables._(this.identity, this.host);

  /// The identity variables, which the agent is given.
  final Map<String, String> identity;

  /// The variables the host sets for the session itself, which layers read and the host does not hand on again.
  final Map<String, String> host;

  /// [outer], with this session's variables first.
  String? Function(String) lookup(String? Function(String) outer) =>
      (name) => identity[name] ?? host[name] ?? outer(name);
}

/// The kind and provider of an account, in the words the identity variables use, from the account kind's id and the standards
/// it provides. [harness] is the harness's AOS id, for the program's own login. A kind this does not recognise has none.
(String?, String?) accountWords(
  String kind,
  List<String> provides,
  String? harness,
) {
  // The vendor a name belongs to: an agent's id, a standard it owns, its AOS id or the vendor's own name.
  String? providerOf(String name) => providerCatalog.resolve(name)?.vendor;
  if (providerCatalog.gatewayVendor(kind) case final vendor?) return ('local', vendor);
  final byPrefix = [
    for (final each in providerCatalog.providers)
      if (each.vendor != null) ...[
        if (kind.startsWith(each.vendor!)) each.vendor!,
        if (kind.startsWith(each.id)) each.vendor!,
      ],
  ].firstOrNull;
  final provider =
      byPrefix ??
      (kind == 'harness-login' && harness != null ? providerOf(harness) : null) ??
      provides.map(providerOf).nonNulls.firstOrNull;
  final category = kind == 'harness-login' || kind.endsWith('-login')
      ? 'subscription'
      : kind.contains('api-key')
      ? 'api-key'
      : kind.contains('auth-token') || kind.contains('gateway')
      ? 'gateway'
      : null;
  return (category, provider);
}
