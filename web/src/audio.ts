/** Audio is the browser's side of voice: recording one dictated turn, and playing a reply. */

import { ApiError } from "./api";

/** RATE matches the server's speech stream: mono 16-bit little-endian PCM. */
const RATE = 24000;

/** Recording is a microphone capture in progress. */
export type Recording = {
  /** stop ends the capture and returns what was recorded. */
  stop: () => Promise<Blob>;
  /** cancel ends the capture and discards it. */
  cancel: () => void;
};

/** Record opens the microphone and starts capturing in the browser's own format. */
export async function record(): Promise<Recording> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { echoCancellation: true, noiseSuppression: true },
  });
  const recorder = new MediaRecorder(stream);
  const chunks: Blob[] = [];
  recorder.ondataavailable = (e) => chunks.push(e.data);
  const release = () => stream.getTracks().forEach((track) => track.stop());
  recorder.start();
  return {
    stop: () =>
      new Promise<Blob>((resolve) => {
        recorder.onstop = () => {
          release();
          resolve(new Blob(chunks, { type: recorder.mimeType || chunks[0]?.type || "audio/webm" }));
        };
        recorder.stop();
      }),
    cancel: () => {
      recorder.onstop = release;
      if (recorder.state !== "inactive") recorder.stop();
      else release();
    },
  };
}

/** Playback is a reply being read aloud. */
export type Playback = {
  stop: () => void;
  /** done settles when the reply has been heard, or stopped; it rejects if it could not play. */
  done: Promise<void>;
};

/** Play fetches a speech stream and schedules each chunk right after the one before it. */
export function play(url: string): Playback {
  const context = new AudioContext();
  const abort = new AbortController();
  let stopped = false;
  let wake = () => {};
  const stop = () => {
    stopped = true;
    abort.abort();
    wake();
  };

  const run = async () => {
    // Created during a click, the context may still start suspended in some browsers.
    await context.resume();
    const response = await fetch(url, { signal: abort.signal });
    if (!response.ok) {
      const data = await response.json().catch(() => ({}));
      throw new ApiError(data.error ?? "http", data.message ?? `Reading aloud failed (${response.status})`);
    }
    const reader = response.body!.getReader();
    let next = 0;
    let carry: number | null = null;
    let last: AudioBufferSourceNode | null = null;
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      // A chunk may split a sample; its first byte waits for the next chunk.
      let bytes = value;
      if (carry !== null) {
        bytes = new Uint8Array(value.length + 1);
        bytes[0] = carry;
        bytes.set(value, 1);
        carry = null;
      }
      if (bytes.length % 2) {
        carry = bytes[bytes.length - 1];
        bytes = bytes.subarray(0, bytes.length - 1);
      }
      if (!bytes.length) continue;
      const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.length);
      const samples = new Float32Array(bytes.length / 2);
      for (let i = 0; i < samples.length; i++) samples[i] = view.getInt16(i * 2, true) / 32768;
      const buffer = context.createBuffer(1, samples.length, RATE);
      buffer.copyToChannel(samples, 0);
      const source = context.createBufferSource();
      source.buffer = buffer;
      source.connect(context.destination);
      // After a gap the next chunk starts shortly after now instead of in the past.
      next = Math.max(next, context.currentTime + 0.05);
      source.start(next);
      next += buffer.duration;
      last = source;
    }
    if (last && !stopped) {
      const ended = last;
      await new Promise<void>((resolve) => {
        wake = resolve;
        ended.onended = () => resolve();
      });
    }
  };

  const done = run()
    .catch((error) => {
      if (!stopped) throw error;
    })
    .finally(() => void context.close());
  return { stop, done };
}
