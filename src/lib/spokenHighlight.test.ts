// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { findSentence, rangeAt, sentencesOf, wordSpan } from "./spokenHighlight";

function el(html: string): HTMLElement {
  const d = document.createElement("div");
  d.innerHTML = html;
  document.body.appendChild(d); // ranges are only made for text that is on the page
  return d;
}

describe("sentencesOf", () => {
  const root = el(
    "<p>A <strong>skill</strong> is a folder. It does not access data.</p><pre>let x = 1;</pre><ul><li>First item</li><li> </li></ul>",
  );
  const s = sentencesOf(root);
  it("splits blocks into sentences and skips code and empty blocks", () => {
    expect(s.map((x) => x.text)).toEqual(["A skill is a folder.", "It does not access data.", "First item"]);
  });
  it("maps a sentence back to the DOM, across inline tags", () => {
    expect(rangeAt(s[0].map, s[0].start, s[0].start + s[0].text.length)?.toString()).toBe("A skill is a folder.");
    expect(rangeAt(s[1].map, s[1].start, s[1].start + s[1].text.length)?.toString()).toBe("It does not access data.");
  });
  it("maps each spoken word (char index in the sentence) to its text", () => {
    const words = [...s[0].text.matchAll(/\S+/g)].map((m) => {
      const w = wordSpan(s[0].text, m.index, m[0].length)!;
      return rangeAt(s[0].map, s[0].start + w.start, s[0].start + w.end)?.toString();
    });
    expect(words).toEqual(["A", "skill", "is", "a", "folder"]);
    // the word inside <strong> stays inside it
    const w = wordSpan(s[0].text, 2, 5)!;
    const r = rangeAt(s[0].map, s[0].start + w.start, s[0].start + w.end)!;
    expect(r.startContainer.parentElement?.tagName).toBe("STRONG");
    expect(r.endContainer.parentElement?.tagName).toBe("STRONG");
  });
});

describe("wordSpan", () => {
  it("drops punctuation and works without a length", () => {
    expect(wordSpan("a task.", 2, 5)).toEqual({ start: 2, end: 6 });
    expect(wordSpan("Say: \"hello\" now", 5, 0)).toEqual({ start: 6, end: 11 });
    expect(wordSpan("x", 5, 1)).toBeNull();
    expect(wordSpan("a — b", 2, 1)).toBeNull();
  });
});

describe("findSentence", () => {
  it("finds a repeated sentence after the cursor", () => {
    const b = el("<span>Good.</span> <span>Please</span> <span>say:</span> <span>Good.</span>");
    const first = findSentence(b, "Good.", 0)!;
    const second = findSentence(b, "Good.", first.start + 5)!;
    expect([first.start, second.start]).toEqual([0, 18]);
    expect(rangeAt(second.map, second.start, second.start + 5)?.toString()).toBe("Good.");
    expect(findSentence(b, "Missing.", 0)).toBeNull();
    expect(findSentence(b, "Good.", 99)?.start).toBe(0);
  });
});
