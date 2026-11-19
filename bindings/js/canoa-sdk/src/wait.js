// What a provider does while a program it started runs: it looks at the program between waits, and lets the host's other calls through.
import * as guatiao from 'guatiao';

/**
 * Lets the calls queued behind the running one answer now, and tells how many did. The engine that serves a script runs one call at a
 * time, so a call that waits for long (a program to end) calls this between its looks, or a `cancel` of what it waits for, or any other
 * provider's call, waits behind it. What those calls change the waiting one finds changed: look at its own state again after each.
 * Guatiao's Node binding has no such queue and answers 0.
 * @type {() => number}
 */
export const yieldCalls = /** @type {{ yieldCalls?: () => number }} */ (/** @type {unknown} */ (guatiao)).yieldCalls ?? (() => 0);
