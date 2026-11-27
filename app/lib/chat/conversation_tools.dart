import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../l10n/app_text.dart';
import '../session_store.dart';
import 'conversation_export.dart';
import 'conversation_find.dart';

/// One thing the header and the command palette offer for the conversation shown: its label, icon and what it does.
typedef ConversationTool = (String, IconData, VoidCallback);

/// What a conversation offers to find in, copy and save as Markdown, for [session] under [context].
List<ConversationTool> conversationTools(BuildContext context, ChatSession session, ConversationFinder finder, {VoidCallback? onSide}) => [
  ('Find in conversation', Icons.search, finder.show),
  if (onSide != null) (context.t('side.tool'), Icons.forum_outlined, onSide),
  ('Copy conversation as Markdown', Icons.content_copy, () => unawaited(copyConversationMarkdown(context, session))),
  ('Export conversation as Markdown…', Icons.download_outlined, () => unawaited(exportConversationMarkdown(context, session))),
];
