/** The memory and its tools, against the real kerno-server and the AI SDK's mock model. */

import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { generateText, isStepCount } from 'ai';
import { MockLanguageModelV4 } from 'ai/test';
import { KernoClient } from 'kerno-client';

import { kernoMemory } from '../src/index.ts';
import { startServer, stopServer, withServer } from './server.ts';

test('recall finds memories by keywords, best first, at most limit, and only those of its type', () =>
  withServer(async (kerno) => {
    const memory = kernoMemory(kerno);
    const tea = await memory.remember('The user likes green tea');
    await memory.remember('The user lives in Lisbon');
    await memory.remember('Green tea tea tea is the favourite drink of the user');
    await kerno.createUnit('note', 'a note about green tea that is not a memory');

    const found = await memory.recall('green tea');
    assert.equal(found.length, 2);
    assert.deepEqual(found.map((m) => m.text).sort(), ['Green tea tea tea is the favourite drink of the user', 'The user likes green tea']);
    assert.ok(found.some((m) => m.id === tea.id));
    assert.equal((await memory.recall('green tea', 1)).length, 1);
    assert.deepEqual((await memory.recall('lisbon')).map((m) => m.text), ['The user lives in Lisbon']);
    assert.deepEqual(await memory.recall('zebra quartz'), []);
    assert.deepEqual(await memory.recall('!!!'), []);
    // Another unit type is another memory.
    assert.deepEqual(await kernoMemory(kerno, { type: 'facts' }).recall('green tea'), []);
  }));

test('the best match comes first', () =>
  withServer(async (kerno) => {
    const memory = kernoMemory(kerno);
    await memory.remember('tea');
    await memory.remember('coffee coffee tea');
    await memory.remember('coffee');
    assert.deepEqual((await memory.recall('coffee coffee')).map((m) => m.text), ['coffee', 'coffee coffee tea']);
  }));

test('forget deletes a memory, once, and never a unit of another type', () =>
  withServer(async (kerno) => {
    const memory = kernoMemory(kerno);
    const { id } = await memory.remember('The user likes green tea');
    const note = await kerno.createUnit('note', 'green tea note');
    assert.equal(await memory.forget(note.id), false);
    assert.equal((await kerno.getUnit(note.id)).lifecycle, 'active');
    assert.equal(await memory.forget('not an id'), false);
    assert.equal(await memory.forget('00'.repeat(16)), false);
    assert.equal(await memory.forget(id), true);
    assert.equal(await memory.forget(id), false);
    assert.deepEqual(await memory.recall('tea'), []);
  }));

test('an empty memory is refused before the server is asked', () =>
  withServer(async (kerno) => {
    const memory = kernoMemory(kerno);
    await assert.rejects(memory.remember('  \n'), RangeError);
    assert.equal((await kerno.listUnits()).units.length, 0);
  }));

test('a server that cannot be reached is an error, not an empty memory', async () => {
  const memory = kernoMemory(new KernoClient('http://127.0.0.1:1', { timeoutMs: 2000 }));
  await assert.rejects(memory.recall('anything'), { name: 'Error', message: /cannot reach/ });
  await assert.rejects(memory.forget('00'.repeat(16)), { message: /cannot reach/ });
});

test('memories survive a restart of the server', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'kerno-ai-sdk-restart-'));
  try {
    const first = await startServer(join(dir, 'db'));
    await kernoMemory(new KernoClient(first.url)).remember('The user likes green tea');
    assert.equal(await stopServer(first), 0);
    const second = await startServer(join(dir, 'db'));
    try {
      assert.deepEqual((await kernoMemory(new KernoClient(second.url)).recall('tea')).map((m) => m.text), ['The user likes green tea']);
    } finally {
      second.process.kill('SIGKILL');
      await second.exited;
    }
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

const usage = {
  inputTokens: { total: 1, noCache: 1, cacheRead: 0, cacheWrite: 0 },
  outputTokens: { total: 1, text: 1, reasoning: 0 },
};

/** A model that answers each call with the next of `calls`: a tool call `[name, input]`, or a final text. */
function scriptedModel(...calls: Array<[string, unknown] | string>) {
  let next = 0;
  return new MockLanguageModelV4({
    doGenerate: async () => {
      const call = calls[next++];
      assert.ok(call !== undefined, 'the model was called more often than the test expects');
      return {
        content:
          typeof call === 'string'
            ? [{ type: 'text', text: call }]
            : [{ type: 'tool-call', toolCallId: `call-${next}`, toolName: call[0], input: JSON.stringify(call[1]) }],
        finishReason: { unified: typeof call === 'string' ? 'stop' : 'tool-calls', raw: undefined },
        usage,
        warnings: [],
      };
    },
  });
}

test('an agent remembers, recalls and forgets through generateText', () =>
  withServer(async (kerno) => {
    const memory = kernoMemory(kerno);
    const model = scriptedModel(
      ['remember', { text: 'The user likes green tea' }],
      ['recall', { query: 'tea', limit: 3 }],
      'You like green tea.',
    );
    const result = await generateText({ model, tools: memory.tools(), stopWhen: isStepCount(5), prompt: 'I like green tea' });

    assert.equal(result.text, 'You like green tea.');
    assert.equal(result.steps.length, 3);
    // The model was offered the three tools, each with a JSON schema.
    const offered = model.doGenerateCalls[0]!.tools!.map((t) => t.name).sort();
    assert.deepEqual(offered, ['forget', 'recall', 'remember']);
    // The recall result is what went back to the model in the last step.
    const lastPrompt = JSON.stringify(model.doGenerateCalls[2]!.prompt);
    assert.match(lastPrompt, /The user likes green tea/);
    const [stored] = await memory.recall('tea');
    assert.equal(stored?.text, 'The user likes green tea');

    const forgetting = scriptedModel(['forget', { id: stored!.id }], ['forget', { id: kerno.baseUrl }], 'done');
    const second = await generateText({ model: forgetting, tools: memory.tools(), stopWhen: isStepCount(5), prompt: 'forget it' });
    assert.equal(second.text, 'done');
    assert.deepEqual(await memory.recall('tea'), []);
    // The second forget (an id that is not one) answered { forgotten: false } and did not fail the run.
    assert.match(JSON.stringify(forgetting.doGenerateCalls[2]!.prompt), /"forgotten":false/);
  }));

test('input that does not fit a tool schema never reaches the server', () =>
  withServer(async (kerno, server) => {
    let requests = 0;
    const counting = new KernoClient(server.url, { fetch: (...args) => (requests++, fetch(...args)) });
    const invalid: Array<[string, unknown]> = [
      ['remember', { text: '' }],
      ['recall', { query: '' }],
      ['recall', { query: 'x', limit: 0 }],
      ['recall', { query: 'x', limit: 21 }],
      ['recall', { query: 'x', limit: 1.5 }],
      ['forget', { id: '' }],
    ];
    for (const call of invalid) {
      const model = scriptedModel(call, 'ok');
      await generateText({ model, tools: kernoMemory(counting).tools(), stopWhen: isStepCount(5), prompt: 'go' });
      assert.match(JSON.stringify(model.doGenerateCalls[1]!.prompt), /error/i, JSON.stringify(call));
    }
    assert.equal(requests, 0);
    // The same tools do reach it with valid input.
    await generateText({ model: scriptedModel(['recall', { query: 'x', limit: 20 }], 'ok'), tools: kernoMemory(counting).tools(), stopWhen: isStepCount(5), prompt: 'go' });
    // Two: the first call of a memory turns its type's keyword index on, then searches.
    assert.equal(requests, 2);
    assert.equal((await kerno.listUnits()).units.length, 0);
  }));

test('a memory is found even when other units outrank it in the keyword search', () =>
  withServer(async (kerno) => {
    const memory = kernoMemory(kerno);
    for (let i = 0; i < 10; i++) await kerno.createUnit('note', 'tea tea tea tea tea tea');
    const { id } = await memory.remember('a single tea');
    assert.deepEqual((await memory.recall('tea')).map((m) => m.id), [id]);
  }));

test('the memory turns the keyword index of its type on once', () =>
  withServer(async (kerno, server) => {
    const paths: string[] = [];
    const counting = new KernoClient(server.url, { fetch: (input, init) => (paths.push(`${init?.method} ${new URL(String(input)).pathname}`), fetch(input, init)) });
    const memory = kernoMemory(counting, { type: 'note_to_self' });
    assert.equal(await kerno.getFulltext('note_to_self'), false, 'off by default');
    await memory.remember('the orchid needs water on fridays');
    await memory.remember('the fern likes shade');
    assert.deepEqual((await memory.recall('orchid')).map((m) => m.text), ['the orchid needs water on fridays']);
    assert.equal(await kerno.getFulltext('note_to_self'), true);
    assert.equal(paths.filter((p) => p.endsWith('/fulltext')).length, 1, 'turned on once, not on every call');
  }));
