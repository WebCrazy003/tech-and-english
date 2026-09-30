import { describe, expect, it } from "vitest";
import { wordDiff } from "./diff";

describe("wordDiff", () => {
  it("marks the changed words on both sides", () => {
    const d = wordDiff("Yesterday I deploy the application.", "Yesterday I deployed the application.");
    expect(d.original.filter((w) => w.changed).map((w) => w.text)).toEqual(["deploy"]);
    expect(d.corrected.filter((w) => w.changed).map((w) => w.text)).toEqual(["deployed"]);
  });
  it("handles removed and added words and ignores case and punctuation", () => {
    const d = wordDiff("I am agree with this idea", "I agree with this idea.");
    expect(d.original.filter((w) => w.changed).map((w) => w.text)).toEqual(["am"]);
    expect(d.corrected.every((w) => !w.changed)).toBe(true);
  });
});
