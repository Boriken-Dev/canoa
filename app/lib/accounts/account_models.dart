/// What the host says of accounts: what a provider takes (its kinds and the ways to sign in), and the accounts there are. Nothing
/// here holds a credential; the host keeps those and says only whether an account is signed in.
library;

import '../providers/provider_catalog.dart';

export 'account_meta.dart';

Map<String, Object?> _map(Object? value) => value is Map
    ? {for (final MapEntry(:key, :value) in value.entries) '$key': value}
    : const {};

List<String> _texts(Object? value) => [if (value is List) for (final each in value) if (each is String) each];

String _text(Object? value, [String fallback = '']) => value is String ? value : fallback;

/// What a person reads for a credential standard the host names (`claude-code`, `anthropic-api`): the title the agent that owns it
/// gives. An id no loaded agent owns is shown as the host wrote it.
String standardTitle(String id) => providerCatalog.standardTitle(id);

/// [standards] as one line, in the order the host gave them.
String standardsText(Iterable<String> standards) => standards.map(standardTitle).join(', ');

/// A URL a kind talks to, which the host checks before a session starts.
final class AccountEndpoint {
  const AccountEndpoint({this.key, required this.purpose, this.fallback});

  factory AccountEndpoint.parse(Map<String, Object?> json) =>
      AccountEndpoint(key: json['key'] as String?, purpose: _text(json['purpose']), fallback: json['default'] as String?);

  /// The setting of the kind that holds the URL; null when the harness fixes it.
  final String? key;

  /// `model_api`, `auth` or `proxy`.
  final String purpose;

  /// The URL used when the setting is absent.
  final String? fallback;
}

/// What the host answers when it carries a conversation from one account to another: whether anything moved (accounts that keep
/// their conversations in one place need nothing moved), the id the conversation has now, and how it was carried.
final class AccountMove {
  const AccountMove({required this.moved, required this.session, this.via, this.losses = 0});

  factory AccountMove.parse(Object? json) {
    final map = _map(json);
    final losses = map['losses'];
    return AccountMove(
      moved: map['moved'] == true,
      session: _text(map['session']),
      via: map['via'] is String ? map['via'] as String : null,
      losses: losses is List ? losses.length : 0,
    );
  }

  final bool moved;
  final String session;

  /// `provider` when the agent's own export carried it, `record` when the host's record of it did.
  final String? via;

  /// How many kinds of thing did not carry (a count of the host's loss report).
  final int losses;
}

/// One kind of account a provider takes (`claude-login`, `anthropic-api-key`).
final class AccountKindInfo {
  const AccountKindInfo({
    required this.id,
    required this.title,
    this.description = '',
    this.settings = const {},
    this.endpoints = const [],
    this.rotates = false,
    this.historyPortable = false,
    this.stateDir = false,
    this.renews = false,
    this.notices = const [],
    this.provides = const [],
  });

  factory AccountKindInfo.parse(Map<String, Object?> json) => AccountKindInfo(
    id: _text(json['id']),
    title: _text(json['title'], _text(json['id'])),
    description: _text(json['description']),
    settings: _map(json['settings']),
    endpoints: [if (json['endpoints'] is List) for (final each in json['endpoints'] as List) if (each is Map) AccountEndpoint.parse(_map(each))],
    rotates: json['rotates'] == true,
    historyPortable: json['history_portable'] == true,
    stateDir: json['state_dir'] == true,
    renews: json['renews'] == true,
    notices: _texts(json['notices']),
    provides: _texts(json['provides']),
  );

  final String id, title, description;

  /// The standards an account of this kind provides, which say the agents that can run on it; none when the host does not say.
  final List<String> provides;

  /// A schema of the kind's settings that are not secret (a base URL).
  final Map<String, Object?> settings;
  final List<AccountEndpoint> endpoints;

  /// A renewal replaces the credential, so one session at a time may hold the account.
  final bool rotates;

  /// A session started on one account of this kind resumes on another of the same provider.
  final bool historyPortable;
  final bool stateDir, renews;

  /// What using this kind weakens, to be read before it is added.
  final List<String> notices;

  /// The URL the kind talks to as the model's API, when its settings do not name another.
  String? get modelEndpoint => endpoints.where((each) => each.purpose == 'model_api').firstOrNull?.fallback;
}

/// One way to sign in, which makes an account of the kind it [produces].
final class LoginMethodInfo {
  const LoginMethodInfo({
    required this.id,
    required this.title,
    this.description = '',
    required this.produces,
    this.inputs = const {},
    this.steps = const [],
    this.remote = false,
  });

  factory LoginMethodInfo.parse(Map<String, Object?> json) => LoginMethodInfo(
    id: _text(json['id']),
    title: _text(json['title'], _text(json['id'])),
    description: _text(json['description']),
    produces: _text(json['produces']),
    inputs: _map(json['inputs']),
    steps: _texts(json['steps']),
    remote: json['remote'] == true,
  );

  final String id, title, description, produces;

  /// A schema of what is asked before the sign-in runs (an email, an issuer); empty asks nothing.
  final Map<String, Object?> inputs;

  /// `open_url`, `user_code`, `ask` and `paste_code`, in the order they can come.
  final List<String> steps;
  final bool remote;

  /// The fields [inputs] asks, in its order.
  Map<String, Map<String, Object?>> get inputFields => schemaFields(inputs);
}

/// The properties of an object schema, in order, each as its own map.
Map<String, Map<String, Object?>> schemaFields(Map<String, Object?> schema) => {
  for (final MapEntry(:key, :value) in _map(schema['properties']).entries) key: _map(value),
};

/// One account provider the host carries: an agent's own (Claude's sign-ins) or a maker's (LiteLLM, Foundry Local, an API gateway), with
/// the kinds it makes and its own ways to sign in. Nothing of another provider's is listed under it.
final class AccountProviderInfo {
  const AccountProviderInfo({
    required this.id,
    required this.title,
    this.description = '',
    this.agent,
    this.kinds = const [],
    this.logins = const [],
  });

  /// One object of `accounts/providers`, or null when it is not one.
  static AccountProviderInfo? parse(Object? json) {
    if (json is! Map) return null;
    final id = json['id'];
    if (id is! String) return null;
    return AccountProviderInfo(
      id: id,
      title: _text(json['title'], id),
      description: _text(json['description']),
      agent: json['harness'] as String?,
      kinds: [if (json['kinds'] is List) for (final each in json['kinds'] as List) if (each is Map) _text(each['id'])],
      logins: [if (json['logins'] is List) for (final each in json['logins'] as List) if (each is Map) _text(each['id'])],
    );
  }

  /// The id the host calls it by.
  final String id;

  /// What the provider calls itself.
  final String title;
  final String description;

  /// The agent it belongs to when it is an agent's own; null for a maker, which no agent owns.
  final String? agent;

  /// The ids of the kinds it makes and of its own ways to sign in.
  final List<String> kinds, logins;

  /// The id its mark and its group are named by: the agent's, else its own.
  String get groupId => agent ?? id;

  /// What a person reads: the agent's name for an agent's own, the provider's title for a maker.
  String get label => agent != null ? providerCatalog.titleOf(agent!) : title;
}

/// What one provider takes: [kinds], and the [logins] that make them.
final class AccountCatalog {
  const AccountCatalog({required this.provider, this.kinds = const [], this.logins = const []});

  factory AccountCatalog.parse(Object? json) {
    final map = _map(json);
    return AccountCatalog(
      provider: _text(map['provider']),
      kinds: [if (map['kinds'] is List) for (final each in map['kinds'] as List) if (each is Map) AccountKindInfo.parse(_map(each))],
      logins: [if (map['logins'] is List) for (final each in map['logins'] as List) if (each is Map) LoginMethodInfo.parse(_map(each))],
    );
  }

  final String provider;
  final List<AccountKindInfo> kinds;
  final List<LoginMethodInfo> logins;

  AccountKindInfo? kind(String id) => kinds.where((each) => each.id == id).firstOrNull;
  LoginMethodInfo? login(String id) => logins.where((each) => each.id == id).firstOrNull;

  /// The method the dialog has chosen before the person does. A host on this machine chooses none. Away from it a browser sign-in
  /// comes back to a port of the host, which the person's browser cannot reach, so the first method that works away from the host
  /// and shows a code to confirm (a device code) is chosen; a provider with none leaves the choice to the person.
  LoginMethodInfo? preferred({required bool hostIsHere}) =>
      hostIsHere ? null : logins.where((each) => each.remote && each.steps.contains('user_code')).firstOrNull;
}

/// Where an account stands, from the host's words.
enum AccountStatus {
  /// A credential is stored and no session holds it.
  signedIn,

  /// The login was lost or never made; signing in again replaces it.
  needsSignIn,

  /// A session holds it, and a kind that rotates its credential allows one holder.
  inUse,

  /// The harness's own login, where it runs: nothing is stored here.
  whereItRuns,

  /// The stored record cannot be read.
  unreadable,
}

/// A pool of canoa.cfg, or one the host makes of every account that provides a standard (`any-<standard>`): accounts that provide one
/// credential standard, which a session may run on instead of one account. The members are accounts by name, `local` among them for
/// the login a harness has where it runs.
final class PoolInfo {
  const PoolInfo({required this.name, required this.standard, this.automatic = false, this.accounts = const []});

  /// One object of `pools/list`, or null when it is not one.
  static PoolInfo? parse(Object? json) {
    if (json is! Map || json['name'] is! String || json['standard'] is! String) return null;
    return PoolInfo(
      name: json['name'] as String,
      standard: json['standard'] as String,
      automatic: json['automatic'] == true,
      accounts: _texts(json['accounts']),
    );
  }

  final String name, standard;

  /// Made by the host, in no file: it holds every account that provides the standard and follows them as they come and go.
  final bool automatic;

  /// The members, in the order the pool takes them.
  final List<String> accounts;

  /// Whether a list shows it: the host's own pool of fewer than two accounts cannot rotate, so only the person's own and one of two or
  /// more are shown. A session that names the pool still runs on it.
  bool get listable => !automatic || accounts.length >= 2;
}

/// An account of a pool as one entry offers it.
final class PoolMember {
  const PoolMember({required this.name, this.listed = false});

  final String name;

  /// Whether the entry offers the account by itself, which a connection with no grant for accounts needs to run on it.
  final bool listed;
}

/// A pool as one agent entry offers it to a session.
final class EntryPool {
  const EntryPool({required this.name, required this.standard, this.automatic = false, this.listed = false, this.members = const []});

  /// One object of an entry's `pools`, or null when it is not one.
  static EntryPool? parse(Object? json) {
    if (json is! Map || json['name'] is! String || json['standard'] is! String) return null;
    return EntryPool(
      name: json['name'] as String,
      standard: json['standard'] as String,
      automatic: json['automatic'] == true,
      listed: json['listed'] == true,
      members: [
        if (json['accounts'] is List)
          for (final each in json['accounts'] as List)
            if (each is Map && each['name'] is String) PoolMember(name: each['name'] as String, listed: each['listed'] == true),
      ],
    );
  }

  final String name, standard;
  final bool automatic;

  /// Whether the entry names it in its `accounts`.
  final bool listed;
  final List<PoolMember> members;

  /// Whether a picker shows it: as [PoolInfo.listable], by the accounts the entry can run on.
  bool get listable => !automatic || members.length >= 2;
}

/// A limit the client heard of for an account, which the host holds the account back for: until [resetsAt] (Unix seconds) when the
/// refusal said when, else until the client says it is over.
typedef LimitHint = ({String account, int? resetsAt});

/// The host's answer to which account the next turn of a session runs on (`pools/turn`). For a pool: [account] is the one that
/// takes the turn, [moved] whether that is not the account the session runs on, and [why] `start`, `kept` (the cache is warm),
/// `in_turn` (it lapsed, so the turn is the next account's) or `exhausted` ([left] says what the account it left had used up).
/// For one account the host never moves anything: [exhausted] says it has run out and [alternatives] the entry's other accounts
/// that could take the session, for the person to be offered.
final class TurnAdvice {
  const TurnAdvice({
    required this.account,
    this.pool,
    this.moved = false,
    this.why,
    this.left,
    this.skipped = const [],
    this.exhausted = false,
    this.refillsAt,
    this.alternatives = const [],
  });

  /// One answer of `pools/turn`, or null when it is not one.
  static TurnAdvice? parse(Object? json) {
    if (json is! Map || json['account'] is! String) return null;
    String? text(String key) => json[key] is String ? json[key] as String : null;
    return TurnAdvice(
      account: json['account'] as String,
      pool: text('pool'),
      moved: json['moved'] == true,
      why: text('why'),
      left: text('left'),
      skipped: [
        if (json['skipped'] is List)
          for (final each in json['skipped'] as List)
            if (each is Map && each['account'] is String) (account: each['account'] as String, why: '${each['why'] ?? ''}'),
      ],
      exhausted: json['exhausted'] == true,
      refillsAt: json['refills_at'] is int ? json['refills_at'] as int : null,
      alternatives: [if (json['alternatives'] is List) for (final each in json['alternatives'] as List) if (each is String) each],
    );
  }

  final String account;

  /// The pool the session was started on; null for one account.
  final String? pool;
  final bool moved;
  final String? why, left;

  /// The accounts of the pool that could not take the turn, and why.
  final List<({String account, String why})> skipped;
  final bool exhausted;

  /// Unix seconds, when the host knows when the account's allowance comes back.
  final int? refillsAt;
  final List<String> alternatives;
}

/// The host refused the turn: no account of the pool has usage left. [message] is its words, which say when the soonest allowance
/// refills.
final class TurnRefused implements Exception {
  const TurnRefused(this.message);

  final String message;

  @override
  String toString() => message;
}

/// A session's account has run out and the person may move it: what is offered in the conversation. It moves nothing by itself.
final class AccountOffer {
  const AccountOffer({required this.account, required this.alternatives, this.left, this.refillsAt});

  /// The account the session runs on, which has run out.
  final String account;

  /// The accounts the host says could take the session, at most three.
  final List<String> alternatives;

  /// What the account has used up, in the host's words.
  final String? left;
  final int? refillsAt;
}

/// An agent entry of the host that runs a provider with accounts, as the host lists it: what it offers a client is the host's to say,
/// and a built-in source (Claude, Codex) is one before canoa.cfg names it.
final class AccountEntry {
  const AccountEntry({
    required this.name,
    required this.harness,
    this.accounts = const [],
    this.open = true,
    this.accepts = const [],
    this.usable,
    this.pools = const [],
  });

  /// One object of `accounts/entries`, or null when it is not one.
  static AccountEntry? parse(Object? json) {
    if (json is! Map) return null;
    final [name, harness, accounts] = [json['name'], json['harness'], json['accounts']];
    if (name is! String || harness is! String) return null;
    return AccountEntry(
      name: name,
      harness: harness,
      accounts: [if (accounts is List) for (final each in accounts) if (each is String) each],
      open: json['open'] != false,
      accepts: _texts(json['accepts']),
      usable: json['usable'] is List ? _texts(json['usable']) : null,
      pools: [if (json['pools'] is List) for (final each in json['pools'] as List) ?EntryPool.parse(each)],
    );
  }

  final String name, harness;

  /// The credential standards its agent accepts, the most preferred first; none when the host does not say.
  final List<String> accepts;

  /// The accounts of canoa.cfg a session of it can run on, offered by it or not. Null from a host that does not say, which leaves
  /// the choice to the provider's own accounts.
  final List<String>? usable;

  /// The accounts it offers, the first being the default; a name here may be a pool's.
  final List<String> accounts;

  /// The pools it offers a session, those it names and the host's own `any-<standard>` its harness accepts.
  final List<EntryPool> pools;

  /// Whether every paired key may use it.
  final bool open;
}

/// One account the host lists, with what it says of its credential: canoa.cfg's `[host.accounts]`, and the [automatic] ones.
final class AccountInfo {
  const AccountInfo({
    required this.name,
    required this.provider,
    required this.kind,
    required this.id,
    this.agent,
    this.fits = const [],
    this.status = AccountStatus.signedIn,
    this.stateText = '',
    this.revision = 0,
    this.expiresAt,
    this.types = const [],
    this.automatic = false,
    this.warnings = const [],
    this.via,
    this.note,
  });

  /// One object of `accounts list --format json`, or null when it is not one.
  static AccountInfo? parse(Object? json) {
    final map = _map(json);
    final [name, provider, kind, id] = [map['name'], map['provider'], map['kind'], map['id']];
    if (name is! String || provider is! String || kind is! String || id is! String) return null;
    final state = _text(map['state']);
    return AccountInfo(
      name: name,
      provider: provider,
      agent: map['agent'] as String?,
      fits: _texts(map['fits']),
      kind: kind,
      id: id,
      status: switch (state) {
        'signed in' => AccountStatus.signedIn,
        'needs sign-in' => AccountStatus.needsSignIn,
        'in use' => AccountStatus.inUse,
        'signed in where it runs' => AccountStatus.whereItRuns,
        _ => AccountStatus.unreadable,
      },
      stateText: state,
      revision: map['revision'] is int ? map['revision'] as int : 0,
      expiresAt: map['expires_at'] as String?,
      types: _texts(map['types']),
      automatic: map['automatic'] == true,
      warnings: _texts(map['warnings']),
      via: map['via'] as String?,
      note: map['note'] as String?,
    );
  }

  /// The account's name, the id of the account provider that made it, and its kind.
  final String name, provider, kind;

  /// The agent the provider belongs to when it is an agent's own; null for a maker's account.
  final String? agent;

  /// The agents that can run a session on the account, by id, as the host works them out from its types.
  final List<String> fits;

  /// What the host tells the person of its settings, such as an address that is plain http beyond this computer. Never a refusal.
  final List<String> warnings;

  /// Whether the host makes the account itself: the login a harness has where it runs, called [localAccount] for each harness that
  /// has one. No canoa.cfg names it, so it is neither removed, offered nor signed in again, and its name alone does not tell
  /// one harness's from another's: its [id] does.
  final bool automatic;

  /// How the automatic account signs in when it is an environment key (`environment key (ANTHROPIC_API_KEY)`: the variable's name,
  /// never its value); null for the login the program keeps for itself.
  final String? via;

  /// Why the program's own account provider could not say whether the login is there (a keyring that did not answer, a place it cannot
  /// look at): the account is listed all the same. Null for any other account and for a login that is there.
  final String? note;

  /// The credential standards the account works for (`claude-code`, `anthropic-api`), as its provider answers them for this account.
  final List<String> types;

  /// The id its group and mark are named by: its provider's agent, else the provider.
  String get groupId => agent ?? provider;

  /// The host's id for the account, which usage reports are named by. Opaque: shown nowhere.
  final String id;
  final AccountStatus status;

  /// What the host said, kept for a state this app does not know.
  final String stateText;
  final int revision;
  final String? expiresAt;

  /// What a person reads for [status].
  String get statusLabel => switch (status) {
    AccountStatus.signedIn => 'Signed in',
    AccountStatus.needsSignIn => 'Needs sign-in',
    AccountStatus.inUse => 'In use',
    AccountStatus.whereItRuns => 'Signed in where it runs',
    AccountStatus.unreadable => stateText.isEmpty ? 'Unknown' : stateText,
  };

  /// Whether a session may start on it: a login that is gone, or one a session holds, cannot be chosen.
  bool get usable => status == AccountStatus.signedIn || status == AccountStatus.whereItRuns;
}

/// The automatic account every harness that has a login of its own offers: the login it has where it runs.
const localAccount = 'local';



/// What a session's account holds when the person chose the login the agent has where it runs, not an account of the host. It is
/// sent as an explicit null, which an entry that lists accounts reads as "none", where an absent name means its first.
const machineLogin = '';
