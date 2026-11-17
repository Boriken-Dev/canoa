import '../sources/source_icons.dart';
import 'look.dart';

/// The family the app draws in. With [follow] on and a conversation shown, its agent's vendor picks it (Claude's, or
/// ChatGPT's for Codex); any other agent, and every section but the conversations, keep [chosen].
LookId familyFor({required LookId chosen, required bool follow, required bool inConversations, String? provider}) {
  if (!follow || !inConversations) return chosen;
  return switch (markIdOf(provider)) {
    'claude' => LookId.claude,
    'codex' => LookId.chatgpt,
    _ => chosen,
  };
}
