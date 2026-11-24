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

/** Wav packs 16-bit mono samples between -1 and 1 into a WAV file. */
export function wav(samples: Float32Array, rate: number): Blob {
  const view = new DataView(new ArrayBuffer(44 + samples.length * 2));
  const ascii = (offset: number, text: string) => {
    for (let i = 0; i < text.length; i++) view.setUint8(offset + i, text.charCodeAt(i));
  };
  ascii(0, "RIFF");
  view.setUint32(4, 36 + samples.length * 2, true);
  ascii(8, "WAVE");
  ascii(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  ascii(36, "data");
  view.setUint32(40, samples.length * 2, true);
  samples.forEach((sample, i) => {
    const clamped = Math.max(-1, Math.min(1, sample));
    view.setInt16(44 + i * 2, clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff, true);
  });
  return new Blob([view.buffer], { type: "audio/wav" });
}

/** audioSession is how an iPhone is told what a page's sound is; other browsers lack it. */
const audioSession = (navigator as { audioSession?: { type: string } }).audioSession;
/** reading counts the replies being read aloud outside voice mode. */
let reading = 0;

/** Play fetches a speech stream and schedules each chunk right after the one before it;
 *  onAudio runs once, when the first chunk is scheduled. Without a `shared` context it makes
 *  its own, which must happen during a click; voice mode passes one made when it started. */
export function play(url: string, onAudio?: () => void, shared?: AudioContext): Playback {
  // An iPhone mutes a page's sound along with its ringer unless the page calls it playback.
  // Voice mode is not muted: its microphone makes the page a call.
  if (!shared && audioSession && reading++ === 0) audioSession.type = "playback";
  const context = shared ?? new AudioContext();
  const abort = new AbortController();
  const sources: AudioBufferSourceNode[] = [];
  let stopped = false;
  let wake = () => {};
  const stop = () => {
    stopped = true;
    abort.abort();
    sources.forEach((source) => source.stop());
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
      if (done || stopped) break;
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
      if (!last) onAudio?.();
      last = source;
      sources.push(source);
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
    .finally(() => {
      if (shared) return;
      void context.close();
      // The microphone cannot be used while the page is playback.
      if (audioSession && --reading === 0) audioSession.type = "auto";
    });
  return { stop, done };
}
