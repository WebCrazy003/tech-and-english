import { useState } from "react";
import { splitKeywords } from "../lib/format";
import styles from "./components.module.css";

/** Chip input: type and press Enter or comma to add; Backspace on empty removes the last chip. */
export default function KeywordInput({
  value,
  onChange,
  placeholder,
  label,
}: {
  value: string[];
  onChange: (v: string[]) => void;
  placeholder?: string;
  label: string;
}) {
  const [text, setText] = useState("");
  const commit = (raw: string) => {
    const add = splitKeywords(raw).filter((k) => !value.some((v) => v.toLowerCase() === k.toLowerCase()));
    if (add.length) onChange([...value, ...add]);
    setText("");
  };
  return (
    <div className={styles.kwBox}>
      {value.map((k) => (
        <span key={k} className={styles.kwChip}>
          {k}
          <button className="ghost" aria-label={`Remove ${k}`} onClick={() => onChange(value.filter((v) => v !== k))}>
            ×
          </button>
        </span>
      ))}
      <input
        type="text"
        aria-label={label}
        value={text}
        placeholder={value.length ? "" : placeholder}
        onChange={(e) => {
          const v = e.target.value;
          if (/[,;]/.test(v)) commit(v);
          else setText(v);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            commit(text);
          } else if (e.key === "Backspace" && !text && value.length) {
            onChange(value.slice(0, -1));
          }
        }}
        onBlur={() => text && commit(text)}
      />
    </div>
  );
}
