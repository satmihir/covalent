import { useEffect, useRef, useState } from "react";
import type { Model } from "./api";
import { CheckIcon, ChevronIcon } from "./icons";

type Props = {
  models: Model[];
  /** value is the chat's model, or null when it is not known. */
  value: string | null;
  disabled: boolean;
  onChange: (model: string) => void;
};

export function ModelPicker({ models, value, disabled, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const outside = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false);
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);

  return (
    <div className="picker" ref={root}>
      <button
        className="picker-button"
        disabled={disabled || models.length === 0}
        onClick={() => setOpen(!open)}
        aria-haspopup="listbox"
        aria-expanded={open}
        title={disabled ? "Wait for the reply to finish to change models" : "Choose a model"}
      >
        <span>{value ?? "Model"}</span>
        <ChevronIcon />
      </button>
      {open && (
        <ul className="picker-menu" role="listbox" aria-label="Models">
          {models.map((m) => (
            <li key={m.id}>
              <button
                role="option"
                aria-selected={m.id === value}
                onClick={() => {
                  setOpen(false);
                  if (m.id !== value) onChange(m.id);
                }}
              >
                <span className="picker-text">
                  <span className="picker-id">{m.id}</span>
                  {m.note && <span className="picker-note">{m.note}</span>}
                </span>
                {m.id === value && <CheckIcon />}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
