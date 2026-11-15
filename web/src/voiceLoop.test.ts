import assert from "node:assert/strict";
import { test } from "node:test";
import { VoiceLoop, type Reply, type VoiceDeps, type VoiceStatus } from "./voiceLoop.ts";

/** Deferred is a promise the test settles by hand. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

/** Settle lets the loop react to promises the test just settled. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

/** Fake records what the loop asks of the world and lets the test answer each request. */
function fake() {
  const statuses: VoiceStatus[] = [];
  const transcripts: ReturnType<typeof deferred<string>>[] = [];
  const sends: { text: string; reply: ReturnType<typeof deferred<Reply | null>> }[] = [];
  const speeches: { reply: Reply; onAudio: () => void; done: ReturnType<typeof deferred<void>>; stopped: boolean }[] = [];
  const pending = new Set<string>();
  const cancelled: string[] = [];
  const errors: unknown[] = [];
  const deps: VoiceDeps = {
    transcribe: () => {
      const d = deferred<string>();
      transcripts.push(d);
      return d.promise;
    },
    send: (text) => {
      const reply = deferred<Reply | null>();
      sends.push({ text, reply });
      return reply.promise;
    },
    speak: (reply, onAudio) => {
      const speech = { reply, onAudio, done: deferred<void>(), stopped: false };
      speeches.push(speech);
      return { stop: () => (speech.stopped = true), done: speech.done.promise };
    },
    pending: (id) => pending.has(id),
    cancel: (chatId) => cancelled.push(chatId),
    onStatus: (status) => statuses.push(status),
    onError: (error) => errors.push(error),
  };
  return { loop: new VoiceLoop(deps), statuses, transcripts, sends, speeches, pending, cancelled, errors };
}

const audio = new Float32Array(16);

/** Ask runs one turn up to a reply being spoken, and returns that reply. */
async function ask(f: ReturnType<typeof fake>, text: string, replyId: string): Promise<Reply> {
  void f.loop.heard(audio);
  f.transcripts.at(-1)!.resolve(text);
  await settle();
  const reply = { chatId: "chat", replyId };
  f.pending.add(replyId);
  f.sends.at(-1)!.reply.resolve(reply);
  await settle();
  return reply;
}

test("a turn is heard, sent, read aloud as it arrives, and then the loop listens again", async () => {
  const f = fake();
  const reply = await ask(f, "what rises?", "r1");
  assert.deepEqual(f.sends.map((s) => s.text), ["what rises?"]);
  assert.deepEqual(f.speeches.map((s) => s.reply), [reply]);
  assert.equal(f.loop.status, "thinking");
  f.speeches[0].onAudio();
  assert.equal(f.loop.status, "speaking");
  f.pending.delete("r1");
  f.speeches[0].done.resolve();
  await settle();
  assert.deepEqual(f.statuses, ["transcribing", "thinking", "speaking", "listening"]);
  assert.deepEqual(f.cancelled, []);
});

test("silence that transcribes to nothing sends nothing", async () => {
  const f = fake();
  void f.loop.heard(audio);
  f.transcripts[0].resolve("  ");
  await settle();
  assert.equal(f.sends.length, 0);
  assert.equal(f.loop.status, "listening");
});

test("talking over a reply still being written stops it and cancels it", async () => {
  const f = fake();
  await ask(f, "tell me everything", "r1");
  f.speeches[0].onAudio();
  f.loop.speechStarted();
  assert.equal(f.speeches[0].stopped, true);
  assert.deepEqual(f.cancelled, ["chat"]);
  assert.equal(f.loop.status, "listening");
  // The stopped speech finishing later changes nothing.
  f.speeches[0].done.resolve();
  await settle();
  assert.equal(f.speeches.length, 1);
  await ask(f, "actually, just one thing", "r2");
  assert.equal(f.speeches.at(-1)!.reply.replyId, "r2");
});

test("talking over a reply that is finished only stops the speech", async () => {
  const f = fake();
  await ask(f, "hi", "r1");
  f.speeches[0].onAudio();
  f.pending.delete("r1");
  f.loop.speechStarted();
  assert.equal(f.speeches[0].stopped, true);
  assert.deepEqual(f.cancelled, []);
});

test("starting to talk while the last words are transcribed does not drop them", async () => {
  const f = fake();
  void f.loop.heard(audio);
  f.loop.speechStarted();
  f.transcripts[0].resolve("first words");
  await settle();
  assert.deepEqual(f.sends.map((s) => s.text), ["first words"]);
});

test("a message overtaken by a newer utterance has its reply cancelled and never read", async () => {
  const f = fake();
  void f.loop.heard(audio);
  f.transcripts[0].resolve("first");
  await settle();
  void f.loop.heard(audio);
  f.pending.add("r1");
  f.sends[0].reply.resolve({ chatId: "chat", replyId: "r1" });
  await settle();
  assert.deepEqual(f.cancelled, ["chat"]);
  assert.equal(f.speeches.length, 0);
  assert.equal(f.loop.status, "transcribing");
});

test("a speech stream that ends while the reply is rewritten is asked for again, a few times", async () => {
  const f = fake();
  await ask(f, "hi", "r1");
  for (let i = 0; i < 7; i++) {
    f.speeches.at(-1)!.done.resolve();
    await settle();
  }
  assert.equal(f.speeches.length, 6, "the first request and five more");
  assert.equal(f.loop.status, "listening");
});

test("ending voice mode stops the speech and lets the reply finish as text", async () => {
  const f = fake();
  await ask(f, "hi", "r1");
  f.speeches[0].onAudio();
  f.loop.close();
  assert.equal(f.speeches[0].stopped, true);
  assert.deepEqual(f.cancelled, []);
  f.speeches[0].done.resolve();
  await settle();
  assert.equal(f.speeches.length, 1);
});

test("a failure is reported and the loop listens again", async () => {
  const f = fake();
  void f.loop.heard(audio);
  f.transcripts[0].reject(new Error("offline"));
  await settle();
  assert.equal((f.errors[0] as Error).message, "offline");
  assert.equal(f.loop.status, "listening");
  void f.loop.heard(audio);
  f.transcripts[1].resolve("hello");
  await settle();
  f.sends[0].reply.resolve(null);
  await settle();
  assert.equal(f.errors.length, 1, "a failed send was already shown");
  assert.equal(f.loop.status, "listening");
});
