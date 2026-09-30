// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { findKnown } from "./highlights";
import { sentenceAt } from "./selection";
import { textKey } from "../../stores/words";
import { accentOf, pickDefaultVoice, toSentences } from "../../lib/tts";

describe("sentenceAt", () => {
  const text = "Kafka stores events. The model runs inference on your laptop. It is fast.";
  it("returns the sentence around the offset", () => {
    expect(sentenceAt(text, text.indexOf("inference"))).toBe("The model runs inference on your laptop.");
    expect(sentenceAt(text, 0)).toBe("Kafka stores events.");
  });
});

describe("textKey", () => {
  it("matches the Rust rule", () => {
    expect(textKey('  "Inference," ')).toBe("inference");
    expect(textKey("Data   Lakehouse.")).toBe("data lakehouse");
  });
});

describe("findKnown", () => {
  it("underlines words and the longest phrase, case-insensitively", () => {
    const div = document.createElement("div");
    div.innerHTML = "<p>A data lake is not a Data warehouse. Low <b>latency</b>, high throughput.</p>";
    const ranges = findKnown(div, new Set(["data lake", "data", "latency", "trade-off"]));
    expect(ranges.map((r) => r.toString())).toEqual(["data lake", "Data", "latency"]);
  });
  it("stops at the limit", () => {
    const div = document.createElement("div");
    div.textContent = "api ".repeat(50);
    expect(findKnown(div, new Set(["api"]), 10)).toHaveLength(10);
  });
});

describe("tts helpers", () => {
  it("splits sentences and drops markdown", () => {
    expect(toSentences("**Key words**\nThis is about DuckDB. It is fast!")).toEqual(["Key words", "This is about DuckDB.", "It is fast!"]);
  });
  it("prefers premium English voices", () => {
    const v = (name: string, lang: string) => ({ name, lang, voiceURI: name }) as SpeechSynthesisVoice;
    const voices = [v("Thomas", "fr-FR"), v("Samantha", "en-US"), v("Zoe (Premium)", "en-US"), v("Daniel", "en-GB")];
    expect(pickDefaultVoice(voices)?.name).toBe("Zoe (Premium)");
    expect(pickDefaultVoice([v("Daniel", "en-GB"), v("Karen", "en-AU")])?.name).toBe("Daniel");
    expect(accentOf("en-IE")).toBe("Ireland");
    expect(pickDefaultVoice([v("Bubbles", "en-US"), v("Karen", "en-AU")])?.name).toBe("Karen");
    const uri = (name: string, lang: string, voiceURI: string) => ({ name, lang, voiceURI }) as SpeechSynthesisVoice;
    const mac = [
      uri("Shelley", "en-US", "com.apple.eloquence.en-US.Shelley"),
      uri("Samantha", "en-US", "com.apple.voice.compact.en-US.Samantha"),
      uri("Ava", "en-US", "com.apple.voice.enhanced.en-US.Ava"),
    ];
    expect(pickDefaultVoice(mac)?.name).toBe("Ava");
    expect(pickDefaultVoice(mac.slice(0, 2))?.name).toBe("Samantha");
  });
});
