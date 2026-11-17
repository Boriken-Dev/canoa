import 'dart:async';

import 'fixtures/sample_render.dart';
import 'fixtures/vendor_looks_fixture.dart';

/// With CANOA_SAMPLES_DIR set, every golden a test draws is written there with real fonts rather than compared. Every test starts
/// with the families a host lists, as an app connected to one does; a test of the other case clears them first.
Future<void> testExecutable(FutureOr<void> Function() testMain) {
  listCrateLooks();
  return renderSamplesIfAsked(testMain);
}
