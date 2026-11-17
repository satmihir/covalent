import { useEffect, useRef, useState } from "react";
import { api, describe, type Personalization as Fields } from "./api";

/** Personalization is a modal dialog to view and edit what every chat is told about the user. */
export function Personalization({ onClose }: { onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [fields, setFields] = useState<Fields | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    dialog.current?.showModal();
    api.personalization().then(setFields, (e) => setError(describe(e)));
  }, []);

  const save = async () => {
    if (!fields) return;
    setSaving(true);
    try {
      await api.personalize(fields);
      onClose();
    } catch (e) {
      setError(describe(e));
      setSaving(false);
    }
  };

  const field = (key: keyof Fields) => ({
    value: fields?.[key] ?? "",
    disabled: !fields,
    onChange: (e: { target: { value: string } }) => fields && setFields({ ...fields, [key]: e.target.value }),
  });

  return (
    <dialog ref={dialog} className="personalization" onClose={onClose}>
      <form
        method="dialog"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <h2>Personalization</h2>
        <p className="personalization-hint">
          Every chat is told this, including chats already open, with your next message in them.
        </p>
        <label>
          Extra instructions and context for all chats
          <textarea rows={4} maxLength={1000} placeholder="How should replies sound?" {...field("instructions")} />
        </label>
        <label>
          Nickname
          <input maxLength={100} placeholder="What should Covalent call you?" {...field("nickname")} />
        </label>
        <label>
          Occupation
          <input maxLength={200} placeholder="What do you do?" {...field("occupation")} />
        </label>
        <label>
          More about you
          <textarea rows={4} maxLength={1000} placeholder="Interests, values, or preferences" {...field("about")} />
        </label>
        {error && (
          <p className="personalization-error" role="alert">
            {error}
          </p>
        )}
        <div className="personalization-actions">
          <button type="button" className="secondary" onClick={() => dialog.current?.close()}>
            Cancel
          </button>
          <button type="submit" className="primary" disabled={!fields || saving}>
            {saving ? "Saving…" : "Save"}
          </button>
        </div>
      </form>
    </dialog>
  );
}
