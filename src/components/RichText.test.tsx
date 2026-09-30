import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import RichText from "./RichText";

const html = (t: string) => renderToStaticMarkup(<RichText text={t} />);

describe("RichText", () => {
  it("renders paragraphs, bold, italic and code", () => {
    expect(html("Hello **world**.\n\nUse `SELECT` and _care_.")).toBe(
      "<p><span>Hello <strong>world</strong>.</span></p><p><span>Use <code>SELECT</code> and <em>care</em>.</span></p>",
    );
  });

  it("renders lists", () => {
    expect(html("- a\n- b")).toBe("<ul><li>a</li><li>b</li></ul>");
    expect(html("1. one\n2. two")).toBe("<ol><li>one</li><li>two</li></ol>");
  });

  it("keeps line breaks inside a paragraph (key words list)", () => {
    expect(html("**Key words**\nlakehouse — mix of lake and warehouse")).toBe(
      "<p><span><strong>Key words</strong></span><span><br/>lakehouse — mix of lake and warehouse</span></p>",
    );
  });

  it("never renders HTML from the model", () => {
    expect(html("<img src=x onerror=alert(1)>")).toBe("<p><span>&lt;img src=x onerror=alert(1)&gt;</span></p>");
  });
});
