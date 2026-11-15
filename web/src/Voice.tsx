import { MicVAD } from "@ricky0123/vad-web";
import { useEffect, useRef, useState } from "react";
import { api, describe } from "./api";
import { play, wav } from "./audio";
import { CloseIcon, MicIcon, MicOffIcon } from "./icons";
import { VoiceLoop, type Reply, type VoiceStatus } from "./voiceLoop";

/** RATE is the sample rate of the utterances the speech detector hands over. */
const RATE = 16000;

type Props = {
  /** send posts a spoken message and says where its reply will appear, or null on failure. */
  send: (text: string) => Promise<Reply | null>;
  pending: (replyId: string) => boolean;
  cancel: (chatId: string) => void;
  /** searching is true while the reply being waited for searches the web. */
  searching: boolean;
  onError: (text: string) => void;
  onClose: () => void;
};

function label(status: VoiceStatus | "starting", muted: boolean, searching: boolean): string {
  switch (status) {
    case "starting":
      return "Starting the microphone…";
    case "listening":
      return muted ? "Muted" : "Listening";
    case "transcribing":
      return "Transcribing…";
    case "thinking":
      return searching ? "Searching the web…" : "Thinking…";
    case "speaking":
      return "Speaking · tap to interrupt";
  }
}

/** Voice is voice mode: it listens, sends what it hears as a message, and reads the reply aloud,
 *  until closed. The thread above shows the same conversation as text. */
export function Voice({ send, pending, cancel, searching, onError, onClose }: Props) {
  const [status, setStatus] = useState<VoiceStatus | "starting">("starting");
  const [muted, setMuted] = useState(false);
  const [level, setLevel] = useState(0);
  const loop = useRef<VoiceLoop | null>(null);
  const detector = useRef<MicVAD | null>(null);
  // Callbacks change on every render of the app; the loop always calls the latest ones.
  const latest = useRef({ send, pending, cancel, onError, onClose });
  latest.current = { send, pending, cancel, onError, onClose };

  useEffect(() => {
    let closed = false;
    // Made while handling the click that opened voice mode, so browsers let it play later.
    const speakers = new AudioContext();
    const voice = new VoiceLoop({
      transcribe: (audio) => api.transcribe(wav(audio, RATE)),
      send: (text) => latest.current.send(text),
      speak: (reply, onAudio) => play(api.speechUrl(reply.chatId, reply.replyId), onAudio, speakers),
      pending: (replyId) => latest.current.pending(replyId),
      cancel: (chatId) => latest.current.cancel(chatId),
      onStatus: setStatus,
      onError: (error) => latest.current.onError(describe(error)),
    });
    loop.current = voice;
    MicVAD.new({
      model: "v5",
      baseAssetPath: "/vad/",
      onnxWASMBasePath: "/vad/",
      // A pause this long ends a turn; shorter ones are a breath inside it.
      redemptionMs: 800,
      onSpeechRealStart: () => voice.speechStarted(),
      onSpeechEnd: (audio) => void voice.heard(audio),
      onFrameProcessed: (probabilities) => setLevel(probabilities.isSpeech),
    }).then(
      (vad) => {
        if (closed) return void vad.destroy();
        detector.current = vad;
        setStatus("listening");
      },
      (error) => {
        if (closed) return;
        const name = error instanceof DOMException ? error.name : "";
        latest.current.onError(
          name === "NotAllowedError"
            ? "Covalent can't use the microphone. Allow it in this site's browser settings."
            : `Voice mode could not start: ${describe(error)}`,
        );
        latest.current.onClose();
      },
    );
    return () => {
      closed = true;
      voice.close();
      void detector.current?.destroy();
      detector.current = null;
      void speakers.close();
    };
  }, []);

  const toggleMute = () => {
    const vad = detector.current;
    if (!vad) return;
    if (muted) void vad.start();
    else void vad.pause();
    setMuted(!muted);
    setLevel(0);
  };

  const busy = status === "thinking" || status === "speaking";
  const scale = status === "listening" && !muted ? 1 + level * 0.18 : 1;
  return (
    <div className="voice" role="region" aria-label="Voice mode">
      <button
        className={`voice-orb ${status} ${muted ? "muted" : ""}`}
        style={{ transform: `scale(${scale})` }}
        onClick={() => busy && loop.current?.interrupt()}
        aria-label={busy ? "Interrupt" : label(status, muted, searching)}
        disabled={!busy}
      />
      <p className="voice-status" role="status">
        {label(status, muted, searching)}
      </p>
      <div className="voice-controls">
        <button
          className={`voice-button ${muted ? "active" : ""}`}
          onClick={toggleMute}
          disabled={status === "starting"}
          aria-label={muted ? "Unmute" : "Mute"}
          title={muted ? "Unmute" : "Mute"}
        >
          {muted ? <MicOffIcon /> : <MicIcon />}
        </button>
        <button className="voice-button end" onClick={onClose} aria-label="End voice mode" title="End voice mode">
          <CloseIcon />
        </button>
      </div>
    </div>
  );
}
