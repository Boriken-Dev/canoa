/// The base environment of a session: the variables every layer's `${...}` and every `from_env` sees before any layer is
/// applied. The same as `canoa_host::profile`'s `Session`.
///
/// Two kinds go in. The identity variables name what the session runs on, as dotagents stamps them (`AGENTS_HARNESS`,
/// `AGENTS_VENDOR`, `AGENT`, `AGENTS_MODEL`, `AGENTS_PROJECT_ROOT`) with Canoa's own beside them; the agent is given them
/// too, and a layer's own value for one replaces them. The host's own variables are those the host sets for the session
/// itself: a layer reads them and may set its own from them.
library;

/// The harness ids and vendors dotagents' registry uses, by the id Canoa names a harness with. A harness with no vendor of
/// its own has none, and then no `AGENTS_VENDOR`.
const _harnesses = {
  'claude-code': 'anthropic',
  'codex': 'openai',
  'gemini-cli': 'google',
  'antigravity': 'google',
  'pi': '',
  'grok': 'xai',
  'opencode': '',
};

/// The vendor-native variable a harness's running model is read from, in dotagents' registry.
const _modelVariables = {
  'claude-code': 'ANTHROPIC_MODEL',
  'codex': 'OPENAI_MODEL',
  'gemini-cli': 'GEMINI_MODEL',
  'pi': 'PI_MODEL',
};

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

  /// `anthropic`, `openai`, `xai`, `google` or `microsoft-foundry`.
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
      final variable = _modelVariables[harness];
      put('AGENTS_HARNESS', harness);
      put('AGENTS_VENDOR', _harnesses[harness]);
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

  /// Every variable name held.
  Iterable<String> get names => [...identity.keys, ...host.keys];

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
  String? providerOf(String name) => switch (name) {
    'claude-code' || 'anthropic-api' || 'claude' || 'anthropic' => 'anthropic',
    'codex' || 'openai-api' || 'openai' => 'openai',
    'grok' || 'xai-api' || 'xai' => 'xai',
    'gemini-api' || 'antigravity' || 'gemini-cli' || 'google' => 'google',
    'foundry-local' => 'microsoft-foundry',
    _ => null,
  };
  if (kind == 'foundry-local') return ('local', 'microsoft-foundry');
  const prefixes = {
    'anthropic': 'anthropic',
    'claude': 'anthropic',
    'openai': 'openai',
    'codex': 'openai',
    'xai': 'xai',
    'grok': 'xai',
    'gemini': 'google',
    'antigravity': 'google',
  };
  final byPrefix = prefixes.entries
      .where((entry) => kind.startsWith(entry.key))
      .map((entry) => entry.value)
      .firstOrNull;
  final provider =
      byPrefix ??
      (kind == 'harness-login' && harness != null
          ? providerOf(harness)
          : null) ??
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
