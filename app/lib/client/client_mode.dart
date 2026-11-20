// How the page that hosts the app asks for it: the whole app, or the chat alone with the sessions of what the host page is
// showing first. The extension's side panel and an Office task pane both ask for the second.
import 'relation.dart';

/// Which app a page shows, from its address.
///
/// `mode=chat` asks for the chat alone: the conversation and the sessions related to what is in front of the client, with no
/// workspace, terminal, Store or Settings. `relation=<kind>:<key>` (with `scope=` and `label=`) names what is in front when the
/// client knows it at load; a client that learns it later gives the app a [RelationSource] instead. `client=<kind>` says which
/// kind of client it is, which canoa.cfg may give a default loadout of its own.
final class ClientMode {
  const ClientMode({this.chatOnly = false, this.relation, this.client});

  /// The whole app.
  static const full = ClientMode();

  /// The address parameter that picks the chat alone, and its value.
  static const parameter = 'mode', chatValue = 'chat';

  /// The parameters of a relation, which a link to the whole app leaves out.
  static const relationParameters = ['relation', 'scope', 'label'];

  /// The address parameter that says which kind of client asked for the chat.
  static const clientParameter = 'client';

  /// Whether the page shows the chat alone.
  final bool chatOnly;

  /// What was in front of the client when the page loaded, if its address said.
  final Relation? relation;

  /// The kind of client the address names, as written; null when it names none. The host knows which kinds have a loadout of
  /// their own, and treats any other as a label.
  final String? client;

  /// The kind of client this is: the one the address names, else what the relation shows (a webpage's origin is a browser's, a mail
  /// conversation Outlook's); null where neither says, as for an Office document, which a Word and an Excel pane both relate to.
  String? get kind =>
      client ??
      switch (relation?.kind) {
        Relation.originKind => 'browser',
        Relation.conversationKind => 'outlook',
        _ => null,
      };

  /// The mode [address] asks for; the whole app when it asks for none.
  factory ClientMode.fromUri(Uri address) {
    final query = address.queryParameters;
    if (query[parameter] != chatValue) return full;
    return ClientMode(
      chatOnly: true,
      relation: Relation.parse(query['relation'], scope: query['scope'], label: query['label']),
      client: query[clientParameter]?.isEmpty ?? true ? null : query[clientParameter],
    );
  }

  /// [address] as a link to the whole app: its mode and relation dropped, `view=full` set, anything else it carries kept.
  static Uri fullAppOf(Uri address) {
    final query = {
      for (final entry in address.queryParameters.entries)
        if (entry.key != parameter && entry.key != clientParameter && !relationParameters.contains(entry.key)) entry.key: entry.value,
      'view': 'full',
    };
    return address.removeFragment().replace(queryParameters: query);
  }
}
