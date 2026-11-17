/**
 * ACP session/request_permission helpers.
 *
 * This is a client-side option validator, not an authorization mechanism for
 * browser tabs or a substitute for user consent. The caller must authenticate
 * the agent first and present the agent's offered choices to the user.
 */

export function offeredOptions(params) {
  if (!params || typeof params !== "object" ||
      typeof params.sessionId !== "string" || !Array.isArray(params.options) ||
      params.options.length === 0) throw new TypeError("Invalid ACP permission request");
  const ids = new Set();
  return params.options.map((option) => {
    if (!option || typeof option.optionId !== "string" ||
        option.optionId.length === 0 || ids.has(option.optionId) ||
        typeof option.name !== "string" || typeof option.kind !== "string") {
      throw new TypeError("Invalid ACP permission option");
    }
    ids.add(option.optionId);
    return { optionId: option.optionId, name: option.name, kind: option.kind };
  });
}

/** On session cancellation, the ACP response outcome must be cancelled. */
export function cancelPermission() {
  return { outcome: { outcome: "cancelled" } };
}

/**
 * After an explicit approval in the browser UI, return exactly the optionId
 * offered by the verified agent. Unknown, invented or stale options fail closed.
 *
 * The browser's separate local page-tool policy remains in effect.
 */
export function selectPermissionOption({ params, optionId, peerId, trustedPeers }) {
  if (!(trustedPeers instanceof Set) || !trustedPeers.has(peerId)) {
    throw new Error("ACP agent peer not verified");
  }
  const options = offeredOptions(params);
  if (typeof optionId !== "string" || !options.some((option) => option.optionId === optionId)) {
    throw new Error("Permission option was not offered");
  }
  return { outcome: { outcome: "selected", optionId } };
}

/**
 * Explicit deny policy can select only an agent-offered reject_once option.
 * If none is offered, cancel; never forge a reject option or convert a cancel
 * into an authorization.
 */
export function denyPermission(params) {
  const rejected = offeredOptions(params).find((option) => option.kind === "reject_once");
  return rejected
    ? { outcome: { outcome: "selected", optionId: rejected.optionId } }
    : cancelPermission();
}
