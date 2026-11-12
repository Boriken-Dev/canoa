import 'dart:async';

import 'fixtures/sample_render.dart';

/// With CANOA_SAMPLES_DIR set, every golden a test draws is written there with real fonts rather than compared.
Future<void> testExecutable(FutureOr<void> Function() testMain) => renderSamplesIfAsked(testMain);
