import 'package:canoa/mail/mail_accounts.dart';

MailAccountInfo mailAccount(
  String name, {
  String backend = 'ews',
  String? backendTitle,
  String state = 'password stored',
  List<String> details = const [],
  String? server = 'https://mail.example.test/EWS/Exchange.asmx',
  String? email,
  String? user,
}) => MailAccountInfo(
      name: name,
      backend: backend,
      backendTitle: backendTitle,
      state: state,
      details: details,
      server: server,
      email: email,
      user: user,
    );

/// Default test schemas matching `canoa-microsoft-mail`'s `ews`, `graph`, and `outlook` backends.
const defaultMailBackends = <MailBackendInfo>[
  MailBackendInfo(
    id: 'ews',
    title: 'Exchange Server',
    description: 'Connect to Exchange Web Services (EWS) with Basic sign-in or the Windows logon.',
    noSecretState: 'signs in with the Windows logon',
    schema: {
      'type': 'object',
      'properties': {
        'server': {'type': 'string', 'title': 'Server'},
        'email': {'type': 'string', 'title': 'Email address'},
        'user': {'type': 'string', 'title': 'User name'},
        'password': {'type': 'string', 'title': 'Password', 'writeOnly': true, 'x-sensitive': true},
        'autodiscover': {'type': 'string', 'title': 'Autodiscover address', 'x-advanced': true},
        'auth': {
          'type': 'string',
          'title': 'Sign-in',
          'enum': ['basic', 'windows'],
          'x-advanced': true,
        },
      },
    },
    form: {
      'fields': {
        'server': {'placeholder': 'https://mail.example.com/EWS/Exchange.asmx'},
        'email': {'placeholder': 'ana@example.com'},
        'user': {'placeholder': r'EXAMPLE\ana'},
        'password': {'widget': 'password'},
      },
    },
  ),
  MailBackendInfo(
    id: 'graph',
    title: 'Microsoft Graph',
    description: 'Connect to Microsoft 365 using Microsoft Graph and Windows Web Account Manager.',
    noSecretState: 'signed in by Microsoft',
    schema: {
      'type': 'object',
      'properties': {
        'client_id': {'type': 'string', 'title': 'Application (client) ID'},
        'tenant': {'type': 'string', 'title': 'Tenant'},
        'email': {'type': 'string', 'title': 'Email address'},
        'server': {'type': 'string', 'title': 'Graph service address', 'x-advanced': true},
      },
      'required': ['client_id'],
    },
    form: {},
  ),
  MailBackendInfo(
    id: 'outlook',
    title: 'Outlook',
    description: 'Connect to classic Outlook for Windows using your Outlook profile.',
    noSecretState: 'signed in by Outlook',
    schema: {
      'type': 'object',
      'properties': {
        'profile': {'type': 'string', 'title': 'Outlook profile'},
      },
    },
    form: {
      'fields': {
        'profile': {'placeholder': 'current Outlook profile when empty'},
      },
    },
  ),
];

final class FakeMailHost implements MailHost {
  FakeMailHost({
    List<MailAccountInfo> accounts = const [],
    List<MailBackendInfo>? backends,
    this.refuseAdd,
    Map<String, MailTest>? tests,
  }) : accounts = [...accounts],
       backends = backends ?? defaultMailBackends,
       tests = {...?tests};

  final List<MailAccountInfo> accounts;
  final List<MailBackendInfo> backends;
  String? refuseAdd;
  final Map<String, MailTest> tests;
  final List<String> tested = [];
  final List<String> removed = [];
  final List<NewMailAccount> added = [];

  @override
  Future<MailAccountsSnapshot> list() async => MailAccountsSnapshot(
        accounts: [...accounts],
        backends: [...backends],
      );

  @override
  Future<MailAccountInfo> add(NewMailAccount account) async {
    if (refuseAdd case final why?) throw StateError(why);
    added.add(account);
    final info = backends.where((b) => b.id == account.backend).firstOrNull;
    final hasPassword = account.settings['password'] is String && (account.settings['password']! as String).isNotEmpty;
    final state = hasPassword
        ? 'password stored'
        : (info?.noSecretState ?? 'ready');
    final details = [
      for (final key in ['email', 'server', 'user', 'profile', 'host'])
        if (account.settings[key] is String && (account.settings[key]! as String).isNotEmpty)
          account.settings[key]! as String,
    ];
    final made = MailAccountInfo(
      name: account.name,
      backend: account.backend,
      backendTitle: info?.title,
      state: state,
      details: details,
      settings: account.settings,
      server: account.settings['server'] is String ? account.settings['server']! as String : null,
      email: account.settings['email'] is String ? account.settings['email']! as String : null,
      user: account.settings['user'] is String ? account.settings['user']! as String : null,
    );
    accounts.add(made);
    return made;
  }

  @override
  Future<MailTest> test(String name) async {
    tested.add(name);
    return tests[name] ?? const MailTest(ok: true, folders: 7);
  }

  @override
  Future<void> remove(String name) async {
    removed.add(name);
    accounts.removeWhere((each) => each.name == name);
  }
}
