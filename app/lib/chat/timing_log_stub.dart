/// A page has no file to time into.
Future<T> timed<T>(String step, Future<T> Function() run) => run();

/// A page has no file to time into.
void timingMark(String step, int ms) {}

/// A page records no timing.
bool get timingOn => false;
