import 'draft_store.dart';

/// Where nothing is kept on disk, drafts last while the app runs.
DraftBackend platformDraftBackend({String? configPath}) => MemoryDraftBackend();
