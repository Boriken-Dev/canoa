// A usage reader written in JavaScript: how an agent's own quota is read when the agent reports none. An agent entry in canoa.cfg
// that names it (`usage = "listed"`) has the host read the agent's quota through it, with no change to Canoa.
//
// A script has no sockets or files, so what this reader reads is what the agent said when a session opened: the models it lists.
// Each becomes a bucket, full, since the agent this stands for never limits a model. A reader of an agent that serves its figures
// somewhere reads them there, from the processes `members` names.
import { Retry, UsageReader, UsageReading, bucket, failed, reading } from '../../../../bindings/js/canoa-sdk/src/usage.js';

/** One agent's reading: the models its `session/new` answer listed. */
class Listed extends UsageReading {
  #models = [];

  opened(result) {
    let answer;
    try {
      answer = JSON.parse(result);
    } catch {
      return;
    }
    const listed = answer?.models?.availableModels;
    this.#models = Array.isArray(listed)
      ? listed.filter((model) => typeof model?.modelId === 'string').map((model) => ({ id: model.modelId, name: model.name }))
      : [];
  }

  read(root, _tree) {
    if (this.#models.length === 0) return failed('the agent has listed no model yet', Retry.NOT_YET);
    // The agent's own process is always among its members; a table with none says the process is gone.
    if (!this.processes(root).includes(root)) return failed('the agent is not running', Retry.NEVER);
    return reading(
      this.#models.map((model) =>
        bucket(`listed:${model.id}`, { label: model.name, models: [model.id], remaining: 1000, limit: 1000, status: 'ok' }),
      ),
    );
  }
}

/** The reader an entry names as `listed`. */
export class ListedUsage extends UsageReader {
  static reader = 'listed';
  static bucket_prefix = 'listed:';
  // The host reads soon after a session opens and again after a tenth of a second: an example is looked at, not waited for.
  static settle_ms = 100;
  static cache_ms = 100;

  open(members) {
    return new Listed(members);
  }
}
