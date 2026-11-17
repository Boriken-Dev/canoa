// How the page that hosts the app asks for it: the whole app, or the chat alone with the sessions of what the host page is
// showing first. The extension's side panel and an Office task pane both ask for the second.
import 'relation.dart';

/// Which app a page shows, from its address.
///
/// `mode=chat` asks for the chat alone: the conversation and the sessions related to what is in front of the client, with no
/// workspace, terminal, Store or Settings. `relation=<kind>:<key>` (with `scope=` and `label=`) names what is in front when the
/// client knows it at load; a client that learns it later gives the app a [RelationSource] instead.
final class ClientMode {
  const ClientMode({this.chatOnly = false, this.relation});

  /// The whole app.
  static const full = ClientMode();

  /// The address parameter that picks the chat alone, and its value.
  static const parameter = 'mode', chatValue = 'chat';

  /// The parameters of a relation, which a link to the whole app leaves out.
  static const relationParameters = ['relation', 'scope', 'label'];

  /// Whether the page shows the chat alone.
  final bool chatOnly;

  /// What was in front of the client when the page loaded, if its address said.
  final Relation? relation;

  /// The mode [address] asks for; the whole app when it asks for none.
  factory ClientMode.fromUri(Uri address) {
    final query = address.queryParameters;
    if (query[parameter] != chatValue) return full;
    return ClientMode(
      chatOnly: true,
      relation: Relation.parse(query['relation'], scope: query['scope'], label: query['label']),
    );
  }

  /// [address] as a link to the whole app: its mode and relation dropped, `view=full` set, anything else it carries kept.
  static Uri fullAppOf(Uri address) {
    final query = {
      for (final entry in address.queryParameters.entries)
        if (entry.key != parameter && !relationParameters.contains(entry.key)) entry.key: entry.value,
      'view': 'full',
    };
    return address.removeFragment().replace(queryParameters: query);
  }
}
