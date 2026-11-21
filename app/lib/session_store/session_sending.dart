part of '../session_store.dart';

extension SessionSending on SessionStore {
  /// A v2 prompt response confirms insertion, not agent turn completion.
  /// The prompt is never retried automatically after a disconnect. Written while a turn runs, the message is queued instead,
  /// with what goes with it, and sent when the turn ends.
  ///
  /// With [steer], a turn that runs is stopped and the message goes first, as [model.ChatSession.steer] says: this completes once
  /// the agent has the message, and throws, with the message and its attachments back in the draft, when it could not be sent.
  Future<void> sendText(String text, {bool steer = false}) async {
    final session = selected;
    if (session == null) return;
    // A message written while a card waits declines it once and says what to do instead; the turn is not stopped.
    if (session.pendingPermissions.isNotEmpty &&
        (text.trim().isNotEmpty || session.attachments.isNotEmpty)) {
      _refuseOnce(session);
    }
    await sendTextTo(session, text, steer: steer);
  }

  /// [sendText] for [session], which need not be the selected one. With [attached] false what the composer holds for the session
  /// does not go: a message written in another window of it has none of its own.
  Future<void> sendTextTo(
    ChatSession session,
    String text, {
    bool attached = true,
    String? from,
    bool steer = false,
  }) async {
    // A file named by an @ goes only while the message still names it.
    if (attached) {
      session.attachments.removeWhere(
        (item) => item.mention && !text.contains('@${item.name}'),
      );
    }
    final held = attached ? List.of(session.attachments) : <Attachment>[];
    if (text.trim().isEmpty && held.isEmpty) return;
    if (session.status == SessionStatus.preview) {
      sendPreviewText(text);
      return;
    }
    if (isStarting(session)) {
      throw StateError('Its first message is still on its way');
    }
    if (session.isDraft) {
      await _startThenDeliver(session, text);
      return;
    }
    if (_asksPool(session)) await _beforeTurn(session);
    await _send(session, text, held, from: from, steer: steer);
  }

  /// Sends the first queued message of [session] now, and lets the queue go on after it.
  Future<void> sendQueued(ChatSession session) => session.chat.queue.isEmpty
      ? Future.value()
      : session.chat.sendQueued(session.chat.queue.first);

  /// Sends everything queued for [session] now as one message, as many as one message holds, and lets the rest go on after it.
  Future<void> sendMerged(ChatSession session) => session.chat.sendMerged();

  /// Stops the running turn and sends [message], one of the queue, first.
  Future<void> steerQueued(ChatSession session, model.QueuedMessage message) =>
      session.chat.steerQueued(message);

  /// What a queued message's attachments were when the person added them, or null when this window did not add them (another
  /// client's message) and they cannot be put back.
  List<Attachment>? _originOf(model.QueuedMessage message) =>
      switch (message.origin) {
        final List<Object?> list
            when list.every((item) => item is Attachment) =>
          list.cast<Attachment>(),
        _ when message.attachments.isEmpty => const [],
        _ => null,
      };

  /// Joins [message] to the one above it, as [model.ChatSession.mergeQueued] does, keeping what its attachments were. Null when it
  /// was joined, else the reason, in words, why not.
  String? mergeQueued(ChatSession session, model.QueuedMessage message) {
    final index = session.chat.queue.indexOf(message);
    if (index < 1) return 'There is nothing above it to join.';
    final above = session.chat.queue[index - 1];
    final aboveOrigin = _originOf(above), messageOrigin = _originOf(message);
    try {
      final joined = session.chat.mergeQueued(message);
      if (joined != null) {
        joined.origin = aboveOrigin == null || messageOrigin == null
            ? null
            : [
                ...aboveOrigin,
                for (final item in messageOrigin)
                  if (!aboveOrigin.any((other) => other.name == item.name))
                    item,
              ];
      }
      return null;
    } on StateError catch (error) {
      return error.message;
    }
  }

  /// Joins each queued message to the one above it as far as one message holds them. Null when any were joined, else why not.
  String? mergeAllQueued(ChatSession session) {
    var joined = 0;
    String? problem;
    var index = 1;
    while (index < session.chat.queue.length) {
      final why = mergeQueued(session, session.chat.queue[index]);
      if (why == null) {
        joined++;
      } else {
        problem ??= why;
        index++;
      }
    }
    return joined > 0 ? null : problem;
  }

  /// Moves [message] [by] places in the queue: negative toward the front.
  void moveQueued(ChatSession session, model.QueuedMessage message, int by) =>
      session.chat.moveQueued(message, by);

  /// Takes [message] out of the queue for the person to write again: its attachments go back among the draft's, and [into] is
  /// given its text. Null when that was done, else the reason, in words, it stays queued.
  String? takeQueued(
    ChatSession session,
    model.QueuedMessage message, {
    required void Function(String text) into,
  }) {
    if (!session.chat.queue.contains(message)) {
      return 'It has been sent already.';
    }
    final origin = _originOf(message);
    if (origin == null) {
      return 'It came with attachments this window cannot take back.';
    }
    final fresh = [
      for (final item in origin)
        if (!session.attachments.any((other) => other.name == item.name)) item,
    ];
    if (session.attachments.length + fresh.length > maxAttachments) {
      return 'The draft has no room for its attachments.';
    }
    if (!session.chat.unqueue(message)) return 'It has been sent already.';
    session.attachments.addAll(fresh);
    _keepDraft(session);
    _notify();
    into(message.text);
    return null;
  }

  /// Changes the text of a queued message.
  void editQueued(
    ChatSession session,
    model.QueuedMessage message,
    String text,
  ) => session.chat.editQueued(message, text);

  /// Takes one queued message out, or every one.
  void removeQueued(ChatSession session, [model.QueuedMessage? message]) =>
      session.chat.removeQueued(message);

  /// Hands [text] and what goes with it to the conversation, which sends it, or queues it while a turn runs. Each attachment is
  /// checked and read before anything is sent: one the agent cannot be given stops the message, which stays to be changed.
  /// [onSent] is told once the agent has the message, which for a v1 agent is before its turn ends.
  Future<void> _send(
    ChatSession session,
    String text,
    List<Attachment> attached, {
    void Function()? onSent,
    String? from,
    bool steer = false,
  }) async {
    final client = _clients[session.id];
    if (client == null || client.isClosed || session.remoteSessionId == null) {
      throw StateError('A verified ACP peer must be connected');
    }
    final takes = takesOf(session)!;
    final attachments = <AcpAttachment>[
      for (final attachment in attached)
        await toAcp(
          attachment,
          takes,
          embedLimit: embedLimit,
          read: readFile,
          agent: SessionStore._named(session.agent),
        ),
    ];
    _resolveBypass(session);
    final origin = List<Attachment>.unmodifiable(attached);
    if (steer) {
      // The agent is asked to stop first; a message it cannot be asked about stays in the draft.
      final queued = await session.chat.steer(
        text,
        attachments: attachments,
        from: from,
        origin: origin,
      );
      onSent?.call();
      session.attachments.removeWhere(attached.contains);
      session.draftText = '';
      _notify();
      try {
        await queued?.handedOver;
      } catch (_) {
        // It could not be sent after the turn ended: what the person wrote is theirs again.
        for (final item in attached) {
          if (session.attachments.length < maxAttachments &&
              !session.attachments.any((other) => other.name == item.name)) {
            session.attachments.add(item);
          }
        }
        _keepDraft(session);
        _notify();
        rethrow;
      }
      return;
    }
    await session.chat.send(
      text,
      attachments: attachments,
      from: from,
      origin: origin,
    );
    onSent?.call();
    session.attachments.removeWhere(attached.contains);
    session.draftText = '';
    _notify();
  }

  /// Whether anything can go with this session's messages: a draft's, or one whose agent this app reaches.
  bool canAttach(ChatSession session) {
    if (session.isDraft) return true;
    final client = _clients[session.id];
    return client != null && !client.isClosed;
  }

  /// Why [attachment] cannot go with [session]'s next message, or null; known only once its agent, or its source's last
  /// time, has said what it takes.
  String? refusalOf(ChatSession session, Attachment attachment) {
    final takes = takesOf(session);
    if (takes == null) return null;
    final (route, why) = routeOf(
      attachment,
      takes,
      embedLimit: embedLimit,
      agent: SessionStore._named(session.agent),
    );
    return route == AttachmentRoute.refused ? why : null;
  }

  /// How [attachment] will reach [session]'s agent when it is not as it is, such as a link in place of an embedded file; null
  /// when it goes as it is, or that is not known yet.
  String? routeNote(ChatSession session, Attachment attachment) {
    final takes = takesOf(session);
    return takes == null
        ? null
        : routeOf(
            attachment,
            takes,
            embedLimit: embedLimit,
            agent: SessionStore._named(session.agent),
          ).$2;
  }

  String? _add(ChatSession session, Attachment attachment) {
    if (!canAttach(session)) return 'This conversation has no agent.';
    if (session.attachments.length >= maxAttachments) {
      return 'That is as many as one message can take.';
    }
    if (session.attachments.any((item) => item.name == attachment.name)) {
      return '${attachment.name} is already there.';
    }
    final refused = refusalOf(session, attachment);
    if (refused != null) return '$refused.';
    session.attachments.add(attachment);
    _keepDraft(session);
    _notify();
    return null;
  }

  /// Takes what a person chose or pasted, by its bytes, to go with their next message: an image, or text. With [path], a file
  /// on this computer that the agent can be given a link to. What is wrong with it, for them to read, or null.
  String? addAttachment(
    ChatSession session,
    String name,
    List<int> bytes, {
    String? mimeType,
    String? path,
  }) {
    if (bytes.isEmpty) return '$name is empty.';
    final image = imageTypeOf(name, mimeType);
    if (image != null) {
      if (bytes.length > maxAttachedImage) {
        return '$name is larger than an image can be here.';
      }
      return _add(session, Attachment.image(name, image, bytes, path: path));
    }
    if (path != null) {
      return _add(
        session,
        Attachment.file(name, path, bytes.length, mimeType: mimeType),
      );
    }
    if (bytes.length > maxAttachedText) {
      return '$name is larger than a file can be here.';
    }
    final String text;
    try {
      text = utf8.decode(bytes);
    } on FormatException {
      return '$name is not text, and only text files and images can be sent.';
    }
    return _add(session, Attachment.text(name, text, mimeType: mimeType));
  }

  /// Puts what a speech service said about a dictation ([notes], its front matter) with [session]'s next message, as a small file
  /// the agent reads; the person sees it among the attachments and can take it off. Why it cannot go, for them to read, or null.
  String? addDictationNotes(ChatSession session, Map<String, Object?> notes) {
    var number = 1;
    while (session.attachments.any((item) => item.name == dictationNotesName(number))) {
      number++;
    }
    return _add(session, Attachment.text(dictationNotesName(number), dictationFrontMatter(notes), mimeType: 'text/markdown'));
  }

  /// [addAttachment], except that an image over the cap is first made smaller (a PNG at a 2048 px, else a 1536 px long edge)
  /// and added as that, under the same name with a `.png` ending; an image that fits, and anything else, goes as it is. What
  /// is wrong, for them to read, when it still cannot be added.
  Future<String?> addAttachmentFitted(
    ChatSession session,
    String name,
    List<int> bytes, {
    String? mimeType,
    String? path,
  }) async {
    if (bytes.length <= maxAttachedImage ||
        imageTypeOf(name, mimeType) == null) {
      return addAttachment(
        session,
        name,
        bytes,
        mimeType: mimeType,
        path: path,
      );
    }
    final fitted = await fitImage(bytes);
    if (fitted == null) {
      return '$name is larger than an image can be here, and could not be made smaller.';
    }
    final dot = name.lastIndexOf('.');
    return addAttachment(
      session,
      '${dot > 0 ? name.substring(0, dot) : name}.png',
      fitted.bytes,
      mimeType: 'image/png',
      path: path,
    );
  }

  /// Takes the file at [path] on this computer, [size] bytes, read only when it is sent: embedded where the agent takes files
  /// and it is small enough text, else linked.
  String? addFile(
    ChatSession session,
    String name,
    String path,
    int size, {
    String? mimeType,
  }) {
    if (size == 0) return '$name is empty.';
    return _add(session, Attachment.file(name, path, size, mimeType: mimeType));
  }

  /// Takes the file or folder at [path], which [label] names after an `@` in the message, to go as a link (a small text file as its
  /// contents where the agent takes files). Nothing happens, and
  /// there is nothing to tell, when it is already there.
  String? addMention(
    ChatSession session,
    String label,
    String path,
    int size, {
    bool folder = false,
    bool onAgentHost = false,
  }) {
    if (session.attachments.any((item) => item.mention && item.name == label)) {
      return null;
    }
    return _add(session, Attachment.mention(label, path, size, folder: folder, onAgentHost: onAgentHost));
  }

  /// Takes the conversation [other], which [label] names after an `@` in the message, as text for the agent: its messages in order,
  /// and only the latest ones when they are more than an attachment can hold. Nothing happens when it is already there.
  String? addConversation(ChatSession session, ChatSession other, String label) {
    if (session.attachments.any((item) => item.mention && item.name == label)) return null;
    final parts = [
      for (final message in other.messages)
        if (message.text.trim().isNotEmpty) '${message.role == MessageRole.user ? 'User' : 'Assistant'}: ${message.text.trim()}',
    ];
    if (parts.isEmpty) return 'That conversation has no messages yet.';
    var kept = parts;
    var text = '# ${other.title}\n\n${kept.join('\n\n')}';
    // The latest messages are the ones that matter when it is more than a file can be here.
    while (utf8.encode(text).length > maxAttachedText && kept.length > 1) {
      kept = kept.sublist(1);
      text = '# ${other.title}\n\n(earlier messages left out)\n\n${kept.join('\n\n')}';
    }
    return _add(session, Attachment.conversation(label, text));
  }

  /// Takes the folder at [path] on this computer, which the agent is given a link to.
  String? addFolder(ChatSession session, String name, String path) =>
      _add(session, Attachment.folder(name, path));

  /// Takes pasted [text] as an attachment of its own.
  String? addText(ChatSession session, String text) {
    final lines = '\n'.allMatches(text).length + 1;
    var name = 'Pasted text, $lines line${lines == 1 ? '' : 's'}';
    for (var n = 2; session.attachments.any((item) => item.name == name); n++) {
      name = 'Pasted text $n, $lines line${lines == 1 ? '' : 's'}';
    }
    if (utf8.encode(text).length > maxAttachedText) {
      return 'That paste is larger than a file can be here.';
    }
    return _add(session, Attachment.text(name, text, mimeType: 'text/plain'));
  }

  void removeAttachment(ChatSession session, Attachment attachment) {
    session.attachments.remove(attachment);
    _keepDraft(session);
    _notify();
  }

  /// Whether [session] ran on one of this computer's own sources and has started, so the host has its history to move.
  bool canMove(ChatSession session) =>
      session.place == SessionPlace.local &&
      !session.isDraft &&
      session.remoteSessionId != null;

}
