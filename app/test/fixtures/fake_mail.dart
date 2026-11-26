import 'package:canoa/desktop/control_client.dart' show ControlFailed;
import 'package:canoa/settings/host_sections.dart';

import 'sections_fixture.dart';

/// One mail account as the `mail/accounts` answer lists it.
Map<String, Object?> mailAccount(
  String name, {
  String backend = 'ews',
  String? backendTitle,
  String state = 'password stored',
  List<String>? details,
  String? server = 'https://mail.example.test/EWS/Exchange.asmx',
  String? email,
  String? user,
}) => {
  'name': name,
  'backend': backend,
  'backend_title': ?backendTitle,
  'state': state,
  'details': details ?? [?email, ?server, ?user],
};

/// Backends with the schemas `canoa-microsoft-mail` describes for `ews`, `graph` and `outlook`.
const defaultMailBackends = <Map<String, Object?>>[
  {
    'id': 'ews',
    'title': 'Exchange Server',
    'description': 'Connect to Exchange Web Services (EWS) with Basic sign-in or the Windows logon.',
    'no_secret_state': 'signs in with the Windows logon',
    'schema': {
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
    'form': {
      'fields': {
        'server': {'placeholder': 'https://mail.example.com/EWS/Exchange.asmx'},
        'email': {'placeholder': 'ana@example.com'},
        'user': {'placeholder': r'EXAMPLE\ana'},
        'password': {'widget': 'password'},
      },
    },
  },
  {
    'id': 'graph',
    'title': 'Microsoft Graph',
    'description': 'Connect to Microsoft 365 using Microsoft Graph and Windows Web Account Manager.',
    'no_secret_state': 'signed in by Microsoft',
    'schema': {
      'type': 'object',
      'properties': {
        'client_id': {'type': 'string', 'title': 'Application (client) ID'},
        'tenant': {'type': 'string', 'title': 'Tenant'},
        'email': {'type': 'string', 'title': 'Email address'},
        'server': {'type': 'string', 'title': 'Graph service address', 'x-advanced': true},
      },
      'required': ['client_id'],
    },
    'form': {},
  },
  {
    'id': 'outlook',
    'title': 'Outlook',
    'description': 'Connect to classic Outlook for Windows using your Outlook profile.',
    'no_secret_state': 'signed in by Outlook',
    'schema': {
      'type': 'object',
      'properties': {
        'profile': {'type': 'string', 'title': 'Outlook profile'},
      },
    },
    'form': {
      'fields': {
        'profile': {'placeholder': 'current Outlook profile when empty'},
      },
    },
  },
];

/// The Mail page as the host lists it once the mail extension is loaded: a library the host carries offers it, so its origin is
/// `bundled`, and its rows are managed through the `mail/*` control methods the extension answers.
Map<String, Object?> mailSectionJson() => {
  'id': 'mail',
  'title': 'Mail',
  'help':
      'An agent can search, read and draft in these mailboxes, and asks you before it moves or changes a message. '
      "It never sends. Passwords stay in this computer's credential store, never in canoa.cfg.",
  'icon': 'mail',
  'group': 'Agents',
  'under': null,
  'keywords': 'email exchange outlook ews mailbox inbox messages password account add remove test sign in',
  'origin': 'bundled',
  'stored': 'provider',
  'schema': null,
  'form': null,
  'collection': {
    'list': 'mail/accounts',
    'add': 'mail/add',
    'remove': 'mail/remove',
    'test': 'mail/test',
    'items': 'accounts',
    'kinds': 'backends',
    'kind_member': 'backend',
    'add_label': 'Add mailbox',
    'empty': 'No mailbox',
    'empty_help': 'Agents have no mail to use until one is added.',
    'kind_label': 'Backend',
    'test_label': 'Test sign-in',
    'remove_note': 'Takes the mailbox out of canoa.cfg and forgets any saved password. Nothing in the mailbox itself changes.',
  },
};

/// What the host's `mail/*` control methods answer, kept in memory: the accounts, what each test finds, and what was asked.
final class FakeMailService {
  FakeMailService({
    List<Map<String, Object?>> accounts = const [],
    List<Map<String, Object?>>? backends,
    this.refuseAdd,
    Map<String, Map<String, Object?>>? tests,
    this.denied = false,
    this.unreachable = false,
  }) : accounts = [...accounts],
       backends = backends ?? [...defaultMailBackends],
       tests = {...?tests};

  final List<Map<String, Object?>> accounts;
  final List<Map<String, Object?>> backends;
  String? refuseAdd;

  /// The host refuses the key: it may not manage the list.
  bool denied;

  /// The host does not answer at all.
  bool unreachable;
  final Map<String, Map<String, Object?>> tests;
  final List<String> tested = [];
  final List<String> removed = [];
  final List<Map<String, Object?>> added = [];

  Future<Object?> call(String method, Map<String, Object?> params) async {
    if (unreachable) throw StateError('no link');
    if (denied) throw const ControlFailed(-32001, 'not allowed');
    switch (method) {
      case 'settings/section_get':
        return {'values': <String, Object?>{}};
      case 'mail/accounts':
        return {'accounts': [...accounts], 'backends': [...backends], 'tools': true};
      case 'mail/add':
        if (refuseAdd case final why?) throw ControlFailed(-32602, why);
        added.add(params);
        final info = backends.where((b) => b['id'] == params['backend']).firstOrNull;
        final password = params['password'];
        String? text(String key) => params[key] is String && (params[key]! as String).isNotEmpty ? params[key]! as String : null;
        final made = mailAccount(
          params['name']! as String,
          backend: params['backend']! as String,
          backendTitle: info?['title'] as String?,
          state: password is String && password.isNotEmpty ? 'password stored' : (info?['no_secret_state'] as String? ?? 'ready'),
          details: [for (final key in ['email', 'server', 'user', 'profile', 'host']) ?text(key)],
          server: null,
        );
        accounts.add(made);
        return {'account': made};
      case 'mail/test':
        final name = params['name']! as String;
        tested.add(name);
        return tests[name] ?? {'ok': true, 'folders': 7, 'message': 'Signed in. 7 folders.'};
      case 'mail/remove':
        final name = params['name']! as String;
        removed.add(name);
        accounts.removeWhere((each) => each['name'] == name);
        return {'removed': true};
    }
    throw ControlFailed(-32601, 'no method $method');
  }

  /// The host's own sections and the mail extension's page, with this service answering the page's methods.
  HostSections sections() {
    final listing = {'sections': [...builtInSectionsJson(), mailSectionJson()]};
    return HostSections(call, slow: call)..adopt(listing);
  }
}
