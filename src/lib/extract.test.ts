// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { extractArticle } from "./extract";

const fixture = (name: string) => readFileSync(resolve(import.meta.dirname, "__fixtures__", name), "utf8");

describe("extractArticle", () => {
  it("finds the article, drops navigation and scripts, and makes links absolute", () => {
    const r = extractArticle(fixture("article.html"), "https://blog.example.com/posts/duckdb");
    expect(r).not.toBeNull();
    expect(r!.text).toContain("in-process analytical database");
    expect(r!.text).not.toContain("Privacy");
    expect(r!.html).not.toContain("<script");
    expect(r!.html).not.toContain("onclick");
    expect(r!.html).toContain('href="https://blog.example.com/docs/storage"');
    expect(r!.html).toContain('src="https://blog.example.com/img/chart.png"');
    expect(r!.text.split("\n\n").length).toBeGreaterThanOrEqual(4);
    expect(r!.text).toContain("Why it matters");
  });

  it("returns null for a page without an article", () => {
    expect(extractArticle(fixture("listing.html"), "https://blog.example.com/")).toBeNull();
  });
});
