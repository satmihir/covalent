/** VoiceLoop holds voice mode's turn-taking rules apart from the microphone, network and
 *  speakers, which it is given, so the rules can be tested on their own. */

export type VoiceStatus = "listening" | "transcribing" | "thinking" | "speaking";

/** Speech is a reply being read aloud. */
export type Speech = { stop: () => void; done: Promise<void> };

export type Reply = { chatId: string; replyId: string };

export type VoiceDeps = {
  transcribe: (audio: Float32Array) => Promise<string>;
  /** send posts a spoken message and says where its reply will appear; null means it failed
   *  and the failure was already shown. */
  send: (text: string) => Promise<Reply | null>;
  /** speak reads a reply aloud as it is written; onAudio runs when the first sound is due. */
  speak: (reply: Reply, onAudio: () => void) => Speech;
  /** pending reports whether a reply is still being written. */
  pending: (replyId: string) => boolean;
  /** cancel stops the chat's reply that is still being written. */
  cancel: (chatId: string) => void;
  onStatus: (status: VoiceStatus) => void;
  onError: (error: unknown) => void;
};

/** RETRIES bounds how often one reply is asked for again after its speech stream ends early,
 *  which happens when the model starts the reply over. */
const RETRIES = 5;

export class VoiceLoop {
  status: VoiceStatus = "listening";
  /** turn changes whenever the user starts over, so late results of an older turn are ignored. */
  private turn = 0;
  private speech: Speech | null = null;
  private reply: Reply | null = null;

  // A plain field, not a parameter property, so Node can run this file by stripping types.
  private deps: VoiceDeps;

  constructor(deps: VoiceDeps) {
    this.deps = deps;
  }

  /** Heard takes one finished utterance; it starts a new turn, replacing any unfinished one. */
  async heard(audio: Float32Array): Promise<void> {
    const turn = this.interrupt();
    this.set("transcribing");
    let text: string;
    try {
      text = (await this.deps.transcribe(audio)).trim();
    } catch (error) {
      return this.fail(turn, error);
    }
    if (turn !== this.turn) return;
    if (!text) return this.set("listening");
    this.set("thinking");
    let reply: Reply | null;
    try {
      reply = await this.deps.send(text);
    } catch (error) {
      return this.fail(turn, error);
    }
    if (!reply) return this.fail(turn, null);
    if (turn !== this.turn) {
      // The user spoke again while this message was on its way; its reply is not wanted.
      if (this.deps.pending(reply.replyId)) this.deps.cancel(reply.chatId);
      return;
    }
    this.reply = reply;
    this.play(turn, reply, RETRIES);
  }

  /** SpeechStarted is the user talking over the assistant, which stops it. */
  speechStarted(): void {
    if (this.status === "thinking" || this.status === "speaking") this.interrupt();
  }

  /** Interrupt stops the current turn's reply, cancelling it if still being written. */
  interrupt(): number {
    this.turn += 1;
    this.speech?.stop();
    this.speech = null;
    if (this.reply && this.deps.pending(this.reply.replyId)) this.deps.cancel(this.reply.chatId);
    this.reply = null;
    this.set("listening");
    return this.turn;
  }

  /** Close stops listening for this loop's results and the speech, and lets a reply still being
   *  written finish as text. */
  close(): void {
    this.turn += 1;
    this.speech?.stop();
    this.speech = null;
    this.reply = null;
  }

  /** Pause stops the speech and ignores late results, as close does, but the loop stays open:
   *  the next utterance starts a turn as usual. A reply still being written finishes as text. */
  pause(): void {
    this.close();
    this.set("listening");
  }

  private play(turn: number, reply: Reply, retries: number): void {
    const speech = this.deps.speak(reply, () => {
      if (turn === this.turn) this.set("speaking");
    });
    this.speech = speech;
    speech.done.then(
      () => {
        if (turn !== this.turn) return;
        if (this.deps.pending(reply.replyId) && retries > 0) return this.play(turn, reply, retries - 1);
        this.speech = null;
        this.reply = null;
        this.set("listening");
      },
      (error) => this.fail(turn, error),
    );
  }

  private fail(turn: number, error: unknown): void {
    if (turn !== this.turn) return;
    if (error !== null) this.deps.onError(error);
    this.speech = null;
    this.reply = null;
    this.set("listening");
  }

  private set(status: VoiceStatus): void {
    if (this.status === status) return;
    this.status = status;
    this.deps.onStatus(status);
  }
}
