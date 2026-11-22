import 'dart:async';

import 'package:flutter/material.dart';

import '../session_store.dart';
import 'conversation_export.dart';

/// One thing the header and the command palette offer for the conversation shown: its label, icon and what it does.
typedef ConversationTool = (String, IconData, VoidCallback);

/// What a conversation offers to copy and save as Markdown, for [session] under [context].
List<ConversationTool> conversationTools(BuildContext context, ChatSession session) => [
  ('Copy conversation as Markdown', Icons.content_copy, () => unawaited(copyConversationMarkdown(context, session))),
  ('Export conversation as Markdown…', Icons.download_outlined, () => unawaited(exportConversationMarkdown(context, session))),
];
