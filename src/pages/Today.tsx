import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { api, type ArticleListItem, type Topic } from "../lib/api";
import { daysAgoIso, scoreLabel, timeAgo } from "../lib/format";
import { useApp } from "../stores/app";
import { toast, toastError } from "../stores/toast";
import ArticleRow from "../components/ArticleRow";
import { Breakdown } from "../components/ScoreChip";
import styles from "./pages.module.css";

function PickCard({
  article,
  why,
  label,
  kind,
}: {
  article: ArticleListItem;
  why?: string;
  label: string;
  kind: "story" | "lesson";
}) {
  const refreshPick = useApp((s) => s.refreshPick);
  const navigate = useNavigate();
  const [finding, setFinding] = useState(false);
  const after = () => void refreshPick();
  const showAnother = () => {
    setFinding(true);
    api
      .nextPick(kind)
      .then(() => refreshPick())
      .catch(toastError)
      .finally(() => setFinding(false));
  };
  // The lesson "why" is "Tutorial · Data Engineering · from a learning source".
  const lessonTopic = kind === "lesson" ? why?.split(" · ")[1] : undefined;
  return (
    <section className={`${styles.card} ${styles.pickCard}`} aria-label={label}>
      <div className={styles.eyebrow}>{label}</div>
      {kind === "lesson" && (
        <div className={styles.meta} style={{ marginTop: 6 }}>
          <span className={styles.learnChip}>📘 {lessonTopic ?? "Lesson"}</span>
        </div>
      )}
      <h2 className={styles.pickTitle}>{article.title}</h2>
      <div className={styles.meta}>
        {kind === "story" && article.primaryTopic && <strong>{article.primaryTopic}</strong>}
        <span>{article.sourceName}</span>
        <span>·</span>
        <span>{timeAgo(article.publishedAt ?? article.discoveredAt)}</span>
        {article.hnPoints != null && <span>· ▲ {article.hnPoints} on Hacker News</span>}
        {article.difficulty && (
          <span>
            · {article.difficulty[0].toUpperCase() + article.difficulty.slice(1)} English
            {article.readingMinutes ? ` · ${article.readingMinutes} min read` : ""}
          </span>
        )}
      </div>
      {article.description && <p className={styles.desc}>{article.description}</p>}
      {why && (
        <div className={kind === "lesson" ? `${styles.why} ${styles.whyLesson}` : styles.why}>
          {kind === "lesson" ? <strong>Why this lesson: </strong> : "💡 "}
          {why}
        </div>
      )}
      <div className={styles.actions}>
        <button className="primary" onClick={() => navigate(`/reader/${article.id}`)}>
          Read
        </button>
        <button className="ghost" onClick={() => api.openArticle(article.id).then(after).catch(toastError)}>
          Browser ↗
        </button>
        <button onClick={() => api.setSaved(article.id, !article.saved).then(after).catch(toastError)}>
          {article.saved ? "★ Saved" : "☆ Save"}
        </button>
        <button onClick={showAnother} disabled={finding} title={`Suggest a different ${kind}`}>
          {finding ? "Finding…" : "Show another →"}
        </button>
        <button
          className="ghost"
          onClick={() =>
            api
              .recordInteraction(article.id, "not_interested")
              .then(() => {
                toast(kind === "lesson" ? "Hidden." : "Hidden. You will see fewer stories like this.");
                after();
              })
              .catch(toastError)
          }
        >
          Not interested
        </button>
      </div>
      {kind === "story" && article.breakdown && (
        <details className={styles.scoreDetails}>
          <summary>Score {scoreLabel(article.score)} / 100</summary>
          <Breakdown b={article.breakdown} />
        </details>
      )}
    </section>
  );
}

function NoLesson({
  beforePickTime,
  pickTime,
  hasLearnTopic,
}: {
  beforePickTime: boolean;
  pickTime?: string;
  hasLearnTopic: boolean;
}) {
  const navigate = useNavigate();
  return (
    <section className={`${styles.card} ${styles.pickCard} ${styles.noLesson}`} aria-label="Today's lesson">
      <div className={styles.eyebrow}>Today's lesson</div>
      {!hasLearnTopic ? (
        <>
          <p>
            No lesson today. Turn on <strong>Learn</strong> for a topic in Settings › Topics.
          </p>
          <p className="faint">Then a tutorial or explainer for that topic is chosen each day.</p>
          <button onClick={() => navigate("/settings/topics")}>Open Topics</button>
        </>
      ) : beforePickTime ? (
        <p>Today's lesson is chosen at {pickTime}, with the story.</p>
      ) : (
        <>
          <p>No lesson today.</p>
          <p className="faint">
            No new tutorial matched your Learn topics. Add a learning source in Settings › News sources, or look in
            Explore › Learning only.
          </p>
        </>
      )}
    </section>
  );
}

export default function Today() {
  const { settings, pick, lesson, preview, newsVersion } = useApp();
  const navigate = useNavigate();
  const [more, setMore] = useState<ArticleListItem[]>([]);
  const [topics, setTopics] = useState<Topic[] | null>(null);

  const loadMore = useCallback(() => {
    api
      .listArticles({ since: daysAgoIso(2) }, null, 8)
      .then((p) => setMore(p.items))
      .catch(toastError);
  }, []);
  useEffect(loadMore, [loadMore, newsVersion]);
  useEffect(() => {
    api
      .listTopics()
      .then(setTopics)
      .catch(() => setTopics(null));
  }, [newsVersion]);

  const hasLearnTopic = topics === null || topics.some((t) => t.learn && t.enabled);
  const shown = [pick?.article.id ?? preview?.id, lesson?.article.id];
  const others = more.filter((a) => !shown.includes(a.id)).slice(0, 5);
  const beforePickTime = settings ? new Date().toTimeString().slice(0, 5) < settings.pickTime : false;

  return (
    <div className={styles.page}>
      <div className={styles.pageHeader}>
        <h1>Today</h1>
        <span className="muted">{new Date().toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" })}</span>
      </div>

      <div className={styles.picks}>
        {pick ? (
          <PickCard kind="story" label="Today's story" article={pick.article} why={pick.why} />
        ) : preview ? (
          <PickCard
            kind="story"
            label={beforePickTime ? `Preview · chosen at ${settings?.pickTime}` : "Best story right now"}
            article={preview}
          />
        ) : (
          <section className={`${styles.card} ${styles.pickCard} ${styles.noLesson}`} aria-label="Today's story">
            <div className={styles.eyebrow}>Today's story</div>
            <p>No story matches your topics yet.</p>
            <p className="faint">Try Refresh news, or add more keywords in Settings › Topics.</p>
            <button onClick={() => navigate("/settings/topics")}>Edit topics</button>
          </section>
        )}
        {lesson ? (
          <PickCard kind="lesson" label="Today's lesson" article={lesson.article} why={lesson.why} />
        ) : (
          <NoLesson beforePickTime={beforePickTime} pickTime={settings?.pickTime} hasLearnTopic={hasLearnTopic} />
        )}
      </div>

      {others.length > 0 && (
        <section className={styles.section}>
          <h2>Also interesting</h2>
          {others.map((a) => (
            <ArticleRow key={a.id} a={a} onChange={loadMore} />
          ))}
        </section>
      )}
    </div>
  );
}
