import { useEffect, useLayoutEffect, useRef } from "react";
import { ArrowUpIcon, StopIcon } from "./icons";

type Props = {
  value: string;
  running: boolean;
  focusKey: string;
  onChange: (value: string) => void;
  onSend: (text: string) => void;
  onStop: () => void;
};

export function Composer({ value, running, focusKey, onChange, onSend, onStop }: Props) {
  const input = useRef<HTMLTextAreaElement>(null);
  const empty = value.trim() === "";

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
