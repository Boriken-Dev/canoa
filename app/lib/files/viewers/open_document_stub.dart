/// Opens [document], one HTML page a viewer drew, in the person's browser; false when this build cannot. A web build cannot: a page
/// opened from the app's own origin would reach what the app keeps in the browser, the pairing key among it.
Future<bool> openDocument(String document, String name) async => false;
