import { Fragment, useEffect, useRef, useState } from "react";
import type { Model } from "./api";
import { ChevronIcon } from "./icons";
import { type Choice, columns, families, label, providerName, providers } from "./modelGrid";

type Props = {
  models: Model[];
  /** value is the chat's model and effort, or null when its model is not known. */
  value: Choice | null;
  disabled: boolean;
  onChange: (model: string, effort: string) => void;
};

/** SHORT fits an effort's name into its column. */
const SHORT: Record<string, string> = { medium: "med" };

/** ModelPicker is a grid: a row for each model family and a column for each reasoning effort,
 *  so one press chooses both. Providers are tabs, and a family's older versions are a press
 *  away on its row. */
export function ModelPicker({ models, value, disabled, onChange }: Props) {
  const [open, setOpen] = useState(false);
  // What the open menu is looking at, apart from what is chosen: a provider, and for a
  // family the version its row shows.
  const [tab, setTab] = useState<string | null>(null);
  const [shown, setShown] = useState<Record<string, string>>({});
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

  const chosen = models.find((m) => m.id === value?.model);
  const tabs = providers(models);
  const provider = tab ?? chosen?.provider ?? tabs[0];
  const rows = families(models, provider);
  const efforts = columns(rows.flatMap((row) => row.versions));
  const choose = (model: Model, effort: string) => {
    setOpen(false);
    if (model.id !== value?.model || effort !== value.effort) onChange(model.id, effort);
  };

  return (
    <div className="picker" ref={root}>
      <button
        className="picker-button"
        disabled={disabled || models.length === 0}
        onClick={() => {
          setTab(null);
          setShown({});
          setOpen(!open);
        }}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={disabled ? "Wait for the reply to finish to change models" : "Choose a model"}
      >
        <span>{label(models, value)}</span>
        <ChevronIcon />
      </button>
      {open && (
        <div className="picker-menu" role="dialog" aria-label="Models">
          {tabs.length > 1 && (
            <div className="picker-tabs" role="tablist">
              {tabs.map((p) => (
                <button key={p} role="tab" aria-selected={p === provider} onClick={() => setTab(p)}>
                  {providerName(p)}
                </button>
              ))}
            </div>
          )}
          <div
            className="picker-grid"
            style={{ gridTemplateColumns: `minmax(96px, auto) repeat(${Math.max(efforts.length, 1)}, minmax(34px, 1fr))` }}
          >
            <span />
            {efforts.length === 0 && <span />}
            {efforts.map((effort) => (
              <span key={effort} className="picker-effort">
                {SHORT[effort] ?? effort}
              </span>
            ))}
            {rows.map((row) => {
              const key = `${provider} ${row.name}`;
              const model =
                row.versions.find((m) => m.id === shown[key]) ??
                row.versions.find((m) => m.id === chosen?.id) ??
                row.versions[0];
              const on = model.id === chosen?.id;
              const next = row.versions[(row.versions.indexOf(model) + 1) % row.versions.length];
              return (
                <Fragment key={key}>
                  <span className={`picker-family${on ? " chosen" : ""}`} title={model.note}>
                    {row.name}
                    {row.versions.length > 1 ? (
                      <button
                        className="picker-version"
                        onClick={() => setShown({ ...shown, [key]: next.id })}
                        aria-label={`${row.name} ${model.version}; show ${next.version}`}
                      >
                        {model.version}
                        <ChevronIcon />
                      </button>
                    ) : (
                      <span className="picker-version">{model.version}</span>
                    )}
                  </span>
                  {model.efforts.length === 0 ? (
                    <button className="picker-standard" aria-pressed={on} onClick={() => choose(model, "")}>
                      standard
                    </button>
                  ) : (
                    efforts.map((effort) =>
                      model.efforts.includes(effort) ? (
                        <button
                          key={effort}
                          className={`picker-cell${effort === model.effort ? " usual" : ""}`}
                          aria-pressed={on && effort === value?.effort}
                          aria-label={`${row.name} ${model.version}, ${effort} effort`}
                          onClick={() => choose(model, effort)}
                        >
                          <span />
                        </button>
                      ) : (
                        <span key={effort} />
                      ),
                    )
                  )}
                </Fragment>
              );
            })}
          </div>
          {chosen && (
            <p className="picker-foot">
              {chosen.id}
              {chosen.note && ` · ${chosen.note}`}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
