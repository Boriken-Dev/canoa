import 'session_ledger.dart';

/// A platform with neither files nor IndexedDB keeps the list while the app runs.
LedgerBackend platformLedgerBackend({String? configPath}) => MemoryLedgerBackend();
