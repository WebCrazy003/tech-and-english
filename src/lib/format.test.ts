import { describe, expect, it } from "vitest";
import { daysAgoIso, shortAge, splitKeywords, timeAgo } from "./format";

const now = new Date("2026-09-29T12:00:00Z");

describe("timeAgo", () => {
  it("formats minutes, hours and days", () => {
    expect(timeAgo("2026-09-29T11:59:50Z", now)).toBe("just now");
    expect(timeAgo("2026-09-29T11:55:00Z", now)).toBe("5 min ago");
    expect(timeAgo("2026-09-29T09:00:00Z", now)).toBe("3 h ago");
    expect(timeAgo("2026-09-28T12:00:00Z", now)).toBe("1 day ago");
    expect(timeAgo("2026-09-26T12:00:00Z", now)).toBe("3 days ago");
  });
  it("handles missing and future values", () => {
    expect(timeAgo(null, now)).toBe("");
    expect(timeAgo("2026-09-30T12:00:00Z", now)).toBe("just now");
  });
});

describe("shortAge", () => {
  it("is compact", () => {
    expect(shortAge("2026-09-29T11:55:00Z", now)).toBe("5m");
    expect(shortAge("2026-09-29T09:00:00Z", now)).toBe("3h");
    expect(shortAge("2026-09-27T12:00:00Z", now)).toBe("2d");
  });
});

describe("splitKeywords", () => {
  it("splits, trims and dedupes", () => {
    expect(splitKeywords(" LLM, llm ;agent\n  tool   calling ,")).toEqual(["LLM", "agent", "tool calling"]);
  });
});

describe("daysAgoIso", () => {
  it("matches the Rust storage format", () => {
    expect(daysAgoIso(1, now)).toBe("2026-09-28T12:00:00Z");
  });
});
