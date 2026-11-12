// The keys that answer a request window: Enter accepts, Escape declines. Enter never answers from a control that uses it itself.
// Nothing here but what the event carries, so it can be tested alone.
const OWN_ENTER = new Set(["INPUT", "TEXTAREA", "SELECT", "BUTTON", "A", "SUMMARY"]);

export function keyAnswer(event) {
  if (event.defaultPrevented || event.isComposing || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return null;
  if (event.key === "Escape") return "decline";
  if (event.key === "Enter" && !OWN_ENTER.has(String(event.target?.tagName ?? "").toUpperCase())) return "accept";
  return null;
}
