/** PAD is how many speech detector frames from before speech begins are sent with it: the
 *  800 ms the detector itself keeps, in 32 ms frames. */
const PAD = 25;

/** WAIT bounds how long a turn's words may take before the whole recording is sent instead. */
const WAIT = 5000;

/** Listen streams voice mode's speech to the server while it is spoken, so the words of a turn
 *  are ready moments after the pause that ends it. Streaming is a shortcut, never a
 *  requirement: when the server cannot be reached, or the words do not come, commit fails and
 *  the caller transcribes the whole recording as before. */
export class Listen {
  private socket: WebSocket | null = null;
  private recent: Float32Array[] = [];
  private streaming = false;
  private turns = 0;
  private waiting = new Map<number, (text: string | null) => void>();
  private url: string;

  constructor(url: string) {
    this.url = url;
    this.connect();
  }

  /** Frame takes each frame the speech detector hears, whether or not it is speech. */
  frame(frame: Float32Array): void {
    if (this.streaming) return this.send(frame);
    this.recent.push(frame);
    if (this.recent.length > PAD) this.recent.shift();
  }

  /** Start begins streaming a turn, from the frames just before the speech. A turn that starts
   *  while the connection is down is not streamed, and the connection is opened again. */
  start(): void {
    if (!this.socket) this.connect();
    if (this.socket?.readyState !== WebSocket.OPEN) return;
    this.streaming = true;
    for (const frame of this.recent.splice(0)) this.send(frame);
  }

  /** Discard drops the turn being streamed, which the detector decided was not speech. */
  discard(): void {
    if (!this.streaming) return;
    this.streaming = false;
    this.socket?.send(JSON.stringify({ type: "clear" }));
  }

  /** Commit ends the turn being streamed and resolves to its words. */
  commit(): Promise<string> {
    const socket = this.socket;
    if (!this.streaming || !socket) return Promise.reject(new Error("this turn was not streamed"));
    this.streaming = false;
    const turn = ++this.turns;
    socket.send(JSON.stringify({ type: "commit", turn }));
    return new Promise((resolve, reject) => {
      const done = (text: string | null) => {
        clearTimeout(timer);
        this.waiting.delete(turn);
        if (text === null) reject(new Error("the streamed words did not come"));
        else resolve(text);
      };
      const timer = setTimeout(() => done(null), WAIT);
      this.waiting.set(turn, done);
    });
  }

  close(): void {
    const socket = this.socket;
    this.socket = null;
    this.fail();
    socket?.close();
  }

  private connect(): void {
    const socket = new WebSocket(this.url);
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data as string) as { turn?: number; text?: string };
      if (message.turn === undefined) return this.fail();
      this.waiting.get(message.turn)?.(message.text ?? null);
    };
    socket.onclose = () => {
      if (this.socket !== socket) return;
      this.socket = null;
      this.fail();
    };
    this.socket = socket;
  }

  /** Fail gives up on every turn still waiting for its words, and on the one being streamed. */
  private fail(): void {
    this.streaming = false;
    for (const done of [...this.waiting.values()]) done(null);
  }

  /** Send passes one frame on as 16-bit PCM, the form the server expects. */
  private send(frame: Float32Array): void {
    const pcm = new Int16Array(frame.length);
    frame.forEach((sample, i) => {
      const clamped = Math.max(-1, Math.min(1, sample));
      pcm[i] = clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff;
    });
    this.socket?.send(pcm.buffer);
  }
}
