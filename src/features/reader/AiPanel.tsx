import { useCallback, useEffect, useRef, useState } from "react";
import { api, type ChatMessage, type QuickAction } from "../../lib/api";
import RichText from "../../components/RichText";
import { useReader } from "../../stores/reader";
import { toastError } from "../../stores/toast";
import { AiDot, AiProblem } from "../ai/AiSetup";
import { useAiStream } from "./useAiStream";
import styles from "./reader.module.css";

const QUICK: { action: QuickAction; label: string }[] = [
  { action: "summarize", label: "Summarize" },
  { action: "key_words", label: "Key words" },
  { action: "explain_simply", label: "Explain simply" },
];

/** AI chat about the open story (SPEC §8.4). History is saved per article. */
export default function AiPanel({ articleId, onClose }: { articleId: number; onClose: () => void }) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [lastAction, setLastAction] = useState<QuickAction | undefined>();
  const { state, run, stop, reset, busy } = useAiStream();
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const quote = useReader((s) => s.quote);
  const setQuote = useReader((s) => s.setQuote);

  const reload = useCallback(() => api.listArticleChat(articleId).then(setMessages).catch(toastError), [articleId]);

  useEffect(() => {
    reset();
    setMessages([]);
    void reload();
  }, [articleId, reload, reset]);

  // "Ask AI about this" from the article.
  useEffect(() => {
    if (!quote) return;
    setInput(`About this part: "${quote.slice(0, 600)}" — `);
    setQuote(null);
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (el) {
        el.focus();
        el.setSelectionRange(el.value.length, el.value.length);
      }
    });
  }, [quote, setQuote]);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [messages, state.text]);

  const send = (msg: { text?: string; action?: QuickAction; retry?: boolean; force?: boolean }) => {
    if (busy) return;
    setLastAction(msg.action);
    void run(
      async (onEvent) => {
        const r = await api.sendArticleChat(articleId, msg, onEvent);
        const um = r.userMessage;
        // The answer can finish (and reload the list) before this call returns: don't add it twice.
        if (um) setMessages((m) => (m.some((x) => x.id === um.id) ? m : [...m, um]));
        return r.jobId;
      },
      // The saved answer replaces the streaming bubble (errors stay visible).
      (final) => void reload().then(() => !final.error && reset()),
    );
  };

  const submit = () => {
    const text = input.trim();
    if (!text) return;
    setInput("");
    send({ text });
  };

  const clear = () => {
    if (!window.confirm("Clear this chat?")) return;
    api
      .clearArticleChat(articleId)
      .then(() => {
        reset();
        setMessages([]);
      })
      .catch(toastError);
  };

  const showStreaming = busy || (state.phase === "done" && !!state.text);

  return (
    <aside className={styles.panel} aria-label="AI chat about this story">
      <header className={styles.panelHeader}>
        <AiDot />
        <strong>AI</strong>
        <span className="spacer" />
        {messages.length > 0 && (
          <button className="ghost" onClick={clear} title="Clear chat">
            Clear
          </button>
        )}
        <button className="ghost" onClick={onClose} aria-label="Hide AI panel" title="Hide">
          ›
        </button>
      </header>

      <div className={styles.chips}>
        {QUICK.map((q) => (
          <button key={q.action} disabled={busy} onClick={() => send({ action: q.action })}>
            {q.label}
          </button>
        ))}
      </div>

      <div className={styles.messages} ref={listRef}>
        {messages.length === 0 && !busy && !state.error && (
          <p className={styles.hint}>Ask anything about this story. You can also select text in the article and press “Ask AI about this”.</p>
        )}
        {messages.map((m) => (
          <div key={m.id} className={m.role === "user" ? styles.userMsg : styles.aiMsg}>
            {m.role === "user" ? m.content : <RichText text={m.content} />}
          </div>
        ))}
        {showStreaming && (
          <div className={`${styles.aiMsg} ${busy ? styles.typing : ""}`}>
            {state.phase === "loading" ? (
              <span className="faint">Starting the AI… (about 10 seconds)</span>
            ) : state.text ? (
              <RichText text={state.text} />
            ) : (
              <span className="faint">Reading the story…</span>
            )}
          </div>
        )}
        {state.error && (
          <AiProblem
            code={state.error.code}
            message={state.error.message}
            retry={(force) => send({ retry: true, action: lastAction, force })}
          />
        )}
      </div>

      <div className={styles.inputRow}>
        <textarea
          ref={inputRef}
          value={input}
          rows={2}
          placeholder="Ask about this story…"
          aria-label="Ask about this story"
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              submit();
            }
          }}
          disabled={busy}
        />
        {busy ? (
          <button onClick={stop} aria-label="Stop">
            ■
          </button>
        ) : (
          <button className="primary" onClick={submit} disabled={!input.trim()} aria-label="Send">
            ↑
          </button>
        )}
      </div>
    </aside>
  );
}
