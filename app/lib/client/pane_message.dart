// What a task pane and the app it frames say to each other. Both live on one origin; each message is a short JSON string.
import 'dart:convert';

import 'relation.dart';

/// The app tells the pane it listens for what is in front of the client, so the pane sends it.
const readyMessage = '{"canoa":"ready"}';

/// The app tells the pane its sign-in ended: the pane shows its own sign-in, since this page cannot be framed.
const signInMessage = '{"canoa":"signin"}';

/// The relation a message from the pane carries, as a record so that a message that names none (`relation: null`, nothing in
/// front) is told from a message that is not one.
typedef RelationMessage = ({Relation? relation});

/// What the pane sent: a relation, or none for nothing in front; null for anything that is not a relation message.
RelationMessage? parseRelationMessage(Object? data) {
  if (data is! String || data.length > 4096) return null;
  final Object? value;
  try {
    value = jsonDecode(data);
  } on FormatException {
    return null;
  }
  if (value is! Map || value['canoa'] != 'relation' || !value.containsKey('relation')) return null;
  final body = value['relation'];
  if (body == null) return (relation: null);
  // A body that is not a relation is not a message: it must not clear what is shown.
  final relation = Relation.fromJson(body);
  return relation == null ? null : (relation: relation);
}
