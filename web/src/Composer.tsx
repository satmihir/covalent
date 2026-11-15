import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { api, describe } from "./api";
import { record, type Recording } from "./audio";
import { ArrowUpIcon, MicIcon, StopIcon } from "./icons";

type Props = {
  value: string;
  running: boolean;
  focusKey: string;
  onChange: (value: string) => void;
  onSend: (text: string) => void;
  onStop: () => void;
  /** voice shows the dictation button. */
  voice: boolean;
  onError: (text: string) => void;
};

type Dictation = "idle" | "starting" | "recording" | "transcribing";

/** MicrophoneError explains a refused or missing microphone in words a person can act on. */
function microphoneError(error: unknown): string {
  const name = error instanceof DOMException ? error.name : "";
  if (name === "NotAllowedError") return "Covalent can't use the microphone. Allow it in this site's browser settings.";
  if (name === "NotFoundError") return "No microphone was found.";
  return describe(error);
}

export function Composer({ value, running, focusKey, onChange, onSend, onStop, voice, onError }: Props) {
  const input = useRef<HTMLTextAreaElement>(null);
  const empty = value.trim() === "";
  const [dictation, setDictation] = useState<Dictation>("idle");
  const recording = useRef<Recording | null>(null);
  // The transcript arrives after typing may have changed the draft, so it joins the latest one.
  const latest = useRef(value);
  latest.current = value;

  useEffect(() => () => recording.current?.cancel(), []);

  const dictate = async () => {
    if (dictation === "recording" && recording.current) {
      const capture = recording.current;
      recording.current = null;
      setDictation("transcribing");
      try {
        const text = await api.transcribe(await capture.stop());
        if (text) onChange(latest.current.trim() ? `${latest.current.trimEnd()} ${text}` : text);
      } catch (error) {
        onError(describe(error));
      } finally {
        setDictation("idle");
        input.current?.focus();
      }
      return;
    }
    if (dictation !== "idle") return;
    setDictation("starting");
    try {
      recording.current = await record();
      setDictation("recording");
    } catch (error) {
      setDictation("idle");
      onError(microphoneError(error));
    }
  };

  // Grow with the text up to the CSS max-height, then scroll inside. Width changes rewrap the
  // text, and the first layout can happen before the column has its final width.
  useLayoutEffect(() => {
    const el = input.current;
    if (!el) return;
    const fit = () => {
      el.style.height = "auto";
      el.style.height = `${el.scrollHeight}px`;
    };
    fit();
    let width = el.clientWidth;
    const observer = new ResizeObserver(() => {
      if (el.clientWidth !== width) {
        width = el.clientWidth;
        fit();
      }
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [value]);

  useEffect(() => {
    input.current?.focus();
  }, [focusKey]);

  const submit = () => {
    if (!empty && !running) onSend(value.trim());
  };

  return (
    <form
      className="composer"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <textarea
        ref={input}
        rows={1}
        value={value}
        placeholder="Message Covalent"
        aria-label="Message"
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            submit();
          }
        }}
      />
      {voice && (
        <button
          type="button"
          className={`mic-button ${dictation}`}
          onClick={() => void dictate()}
          disabled={dictation === "starting" || dictation === "transcribing"}
          aria-label={dictation === "recording" ? "Stop dictating" : "Dictate"}
          title={dictation === "recording" ? "Stop dictating" : dictation === "transcribing" ? "Transcribing…" : "Dictate"}
        >
          {dictation === "recording" ? <StopIcon /> : dictation === "transcribing" ? <span className="spinner" /> : <MicIcon />}
        </button>
      )}
      {running ? (
        <button type="button" className="send-button" onClick={onStop} aria-label="Stop reply" title="Stop">
          <StopIcon />
        </button>
      ) : (
        <button type="submit" className="send-button" disabled={empty} aria-label="Send message" title="Send">
          <ArrowUpIcon />
        </button>
      )}
    </form>
  );
}
