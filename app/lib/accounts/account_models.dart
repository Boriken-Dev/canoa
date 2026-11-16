/// What the host says of accounts: what a provider takes (its kinds and the ways to sign in), and the accounts there are. Nothing
/// here holds a credential; the host keeps those and says only whether an account is signed in.
library;

Map<String, Object?> _map(Object? value) => value is Map
    ? {for (final MapEntry(:key, :value) in value.entries) '$key': value}
    : const {};

List<String> _texts(Object? value) => [if (value is List) for (final each in value) if (each is String) each];

String _text(Object? value, [String fallback = '']) => value is String ? value : fallback;

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
  );

  final String id, title, description;

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

/// What one provider takes: [kinds], and the [logins] that make them.
final class AccountCatalog {
  const AccountCatalog({required this.harness, this.kinds = const [], this.logins = const []});

  factory AccountCatalog.parse(Object? json) {
    final map = _map(json);
    return AccountCatalog(
      harness: _text(map['harness']),
      kinds: [if (map['kinds'] is List) for (final each in map['kinds'] as List) if (each is Map) AccountKindInfo.parse(_map(each))],
      logins: [if (map['logins'] is List) for (final each in map['logins'] as List) if (each is Map) LoginMethodInfo.parse(_map(each))],
    );
  }

  final String harness;
  final List<AccountKindInfo> kinds;
  final List<LoginMethodInfo> logins;

  AccountKindInfo? kind(String id) => kinds.where((each) => each.id == id).firstOrNull;
  LoginMethodInfo? login(String id) => logins.where((each) => each.id == id).firstOrNull;
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

/// An agent entry of the host that runs a provider with accounts, as the host lists it: what it offers a client is the host's to say,
/// and a built-in source (Claude, Codex) is one before canoa.cfg names it.
final class AccountEntry {
  const AccountEntry({required this.name, required this.harness, this.accounts = const [], this.open = true});

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
    );
  }

  final String name, harness;

  /// The accounts it offers, the first being the default.
  final List<String> accounts;

  /// Whether every paired key may use it.
  final bool open;
}

/// One account of canoa.cfg's `[host.accounts]`, with what the host says of its credential.
final class AccountInfo {
  const AccountInfo({
    required this.name,
    required this.harness,
    required this.kind,
    required this.id,
    this.status = AccountStatus.signedIn,
    this.stateText = '',
    this.revision = 0,
    this.expiresAt,
  });

  /// One object of `accounts list --format json`, or null when it is not one.
  static AccountInfo? parse(Object? json) {
    final map = _map(json);
    final [name, harness, kind, id] = [map['name'], map['harness'], map['kind'], map['id']];
    if (name is! String || harness is! String || kind is! String || id is! String) return null;
    final state = _text(map['state']);
    return AccountInfo(
      name: name,
      harness: harness,
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
    );
  }

  final String name, harness, kind;

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

/// The `_meta` key of `initialize` through which the chat names the account a session runs on.
const accountMeta = 'dev.boriken.canoa/account';
