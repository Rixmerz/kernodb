import { tool } from 'ai';
import { ApiError, type KernoClient } from 'kerno-client';
import { z } from 'zod';

export interface KernoMemoryOptions {
  /** The unit type the memories are stored under. The memory reads and deletes only units of this type. */
  type?: string;
}

export interface Memory {
  id: string;
  text: string;
}

export interface KernoMemory {
  /** Stores `text` as a new memory. */
  remember(text: string): Promise<Memory>;
  /** The memories that match `query` by keywords (BM25), best first, at most `limit` (5 by default). */
  recall(query: string, limit?: number): Promise<Memory[]>;
  /** Deletes the memory with this id; `false` when there is no such memory (it is never another type of unit). */
  forget(id: string): Promise<boolean>;
  /** The three operations as tools for `generateText`, `streamText` or an agent: `remember`, `recall` and `forget`. */
  tools(): ReturnType<typeof memoryTools>;
}

const DEFAULT_TYPE = 'memory';
const DEFAULT_LIMIT = 5;
// ponytail: the keyword search cannot be limited to one unit type, so recall reads the best SCAN hits
// of the whole database and keeps those of the memory type. Fine for a database that is mostly
// memories; give the memory a database of its own, or add a type filter to the server, when it is not.
const SCAN = 200;

/**
 * Long-term memory for an AI SDK agent, kept in a kerno-server: one text unit per memory, found by
 * keywords. There are no embeddings: the model asks for what it needs in words.
 *
 * ```ts
 * const memory = kernoMemory(new KernoClient('http://127.0.0.1:8080'));
 * await generateText({ model, tools: memory.tools(), stopWhen: isStepCount(5), prompt });
 * ```
 */
export function kernoMemory(client: KernoClient, options: KernoMemoryOptions = {}): KernoMemory {
  const type = options.type ?? DEFAULT_TYPE;

  const remember = async (text: string): Promise<Memory> => {
    if (text.trim() === '') throw new RangeError('a memory cannot be empty');
    const unit = await client.createUnit(type, text);
    return { id: unit.id, text };
  };

  const recall = async (query: string, limit = DEFAULT_LIMIT): Promise<Memory[]> => {
    const hits = await client.search({ text: query, limit: SCAN, candidates: SCAN });
    const memories: Memory[] = [];
    for (const { unit } of hits) {
      if (unit.type === type && typeof unit.payload === 'string') memories.push({ id: unit.id, text: unit.payload });
    }
    return memories.slice(0, limit);
  };

  const forget = async (id: string): Promise<boolean> => {
    try {
      const unit = await client.getUnit(id);
      if (unit.type !== type || unit.lifecycle === 'deleted') return false;
      await client.deleteUnit(id);
      return true;
    } catch (e) {
      // An id that is not an id (400) or names nothing (404) is not a memory; any other failure is real.
      if (e instanceof ApiError && (e.status === 400 || e.status === 404)) return false;
      throw e;
    }
  };

  return { remember, recall, forget, tools: () => memoryTools({ remember, recall, forget }) };
}

/** The tools of the memory; each one is the operation of the same name, with a schema the model fills in. */
function memoryTools(memory: Pick<KernoMemory, 'remember' | 'recall' | 'forget'>) {
  return {
    remember: tool({
      description: 'Save something worth remembering for later conversations, as one short self-contained statement.',
      inputSchema: z.object({ text: z.string().min(1).describe('What to remember') }),
      execute: ({ text }) => memory.remember(text),
    }),
    recall: tool({
      description: 'Search long-term memory by keywords. Returns the best matches first, each with its id.',
      inputSchema: z.object({
        query: z.string().min(1).describe('Words that the memory is likely to contain'),
        limit: z.number().int().min(1).max(20).optional().describe(`How many memories at most (default ${DEFAULT_LIMIT})`),
      }),
      execute: async ({ query, limit }) => ({ memories: await memory.recall(query, limit) }),
    }),
    forget: tool({
      description: 'Delete a memory by the id that recall returned.',
      inputSchema: z.object({ id: z.string().min(1).describe('The id of the memory') }),
      execute: async ({ id }) => ({ forgotten: await memory.forget(id) }),
    }),
  };
}
