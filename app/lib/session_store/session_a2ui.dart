part of '../session_store.dart';

extension SessionA2ui on SessionStore {
  /// [session]'s A2UI views, made when its first one arrives. What its buttons and its problems say goes to the agent as a message
  /// of its own.
  A2uiSessionViews a2uiOf(ChatSession session) {
    final existing = _a2ui[session.id];
    if (existing != null) return existing;
    final views = _a2ui[session.id] = A2uiSessionViews(limits: a2uiLimits);
    views.outbound = (message) => unawaited(_sendA2ui(session, message));
    views.addListener(() => a2uiChanged.value++);
    views.opened.listen((event) {
      if (!_a2uiOpened.isClosed) _a2uiOpened.add((session, event.view));
    });
    return views;
  }

  /// The views [session] has, none when no agent has drawn one.
  A2uiSessionViews? a2uiViewsOf(ChatSession session) => _a2ui[session.id];

  /// Every view of every session, for the surfaces the shell lists.
  List<(ChatSession, A2uiSessionViews, A2uiView)> a2uiViews() => [
    for (final session in _sessions)
      if (_a2ui[session.id] case final views?)
        for (final view in views.views.values) (session, views, view),
  ];

  /// Draws [view] of the overlay [overlay] in [session], where the view asks to open, with the catalogs the overlay brought. A view
  /// that cannot be drawn is not opened; the answer is why, empty when it was.
  List<String> showOverlayView(ChatSession session, String overlay, OverlayView view, OverlayViews brought) {
    if (!view.drawable) return view.problems;
    final views = a2uiOf(session);
    for (final catalog in brought.catalogs) {
      views.processor.catalogs.register(catalog);
    }
    final messages = scopedMessages(overlay, view.messages);
    // A view shown again starts over, and opens again where it asks.
    final ids = {
      for (final message in messages)
        if (message is Map)
          for (final body in message.values)
            if (body is Map && body['surfaceId'] is String) body['surfaceId']! as String,
    };
    views.carry((
      messages: [
        for (final id in ids) {'version': '1.0', 'deleteSurface': {'surfaceId': id}},
        ...messages,
      ],
      target: view.target.wire,
      title: view.title,
    ));
    return const [];
  }

  /// Tells the agent of [session] what a view did: the protocol's `action` or `error`, in the words of the prompt and as the message
  /// itself in a fenced block, so an agent that takes no files is told as well. A turn that runs queues it.
  Future<void> _sendA2ui(ChatSession session, Map<String, Object?> message) async {
    final action = message['action'];
    final head = action is Map ? 'A2UI action: ${action['name']}' : 'A2UI error: ${(message['error'] as Map?)?['message']}';
    try {
      await _send(session, '$head\n```json\n${jsonEncode(message)}\n```', const []);
    } on Object catch (error) {
      session.chat.note('A view could not tell the agent: $error');
      _notify();
    }
  }
}
