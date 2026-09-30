# P2 T0 — llama-server spike (2026-09-29)

Machine: Mac mini M1, 16 GB, macOS 14.7.
Binary: official llama.cpp nightly **b11256** (`llama-b11256-bin-macos-arm64.tar.gz`, sha256 `8a29bb98…d507e`). It is unpacked to `~/Library/Application Support/com.techenglish.app/bin/llama-b11256/`.

> **Homebrew didn't work.** `brew install llama.cpp` has no bottle for this macOS/brew combination ("Tier 3"). It tried to build from source, and that failed inside the sandbox. **Change to P2 §8.3 (binary resolution):** also look in `<app data>/bin/*/llama-server` (newest folder first).

Model: Qwen3.5-4B Q4_K_M (unsloth, 2.74 GB, Apache-2.0).

## Flags (all present)

`-m`, `--host`, `--port`, `--api-key`, `-c`, `-ngl`, `-np`, `--jinja` (on by default), `--no-webui`, `--reasoning-budget`, `--chat-template-kwargs`, `--cache-prompt` (on by default).

## Behaviour

| Check | Result |
|---|---|
| Start-up to `/health` 200 | **25 s** (cold). While loading, the port refuses connections (curl code 000), not 503 → treat connection-refused as "still starting". |
| Resident memory (RSS) | 384 MB. The weights are memory-mapped and Metal buffers are not counted, so the real footprint is about 3 GB. |
| No API key | 401 ✅ |
| Streaming SSE | `data:` lines, `choices[0].delta.content`, ends with `data: [DONE]` ✅ |
| **Thinking** | Qwen3.5 is a hybrid thinking model. By default it spent the **whole** 200-token budget thinking (`reasoning_content`) and returned **no answer**. With `"chat_template_kwargs": {"enable_thinking": false}` the first token came at **0.27 s**. → The provider MUST send this for models with `thinking: true` in the catalog. It is harmless for other models. |
| JSON schema | `response_format: {type: "json_schema", json_schema: {name, schema}}` works. The output parses, and keys keep **schema order** (`reply` first) ✅ |
| Prompt cache | Turn 1: 1,833 prompt tokens in 9.3 s. Turn 2, same prefix: only 49 new tokens, 0.46 s ✅ |
| Cancel on disconnect | Server log `stop: cancel task`. The next request was answered in 0.41 s ✅ |

## Speed (`llama-bench`, `-ngl 99`)

| Test | tokens/s |
|---|---|
| pp512 | 209 |
| pp2048 | 207 |
| tg128 | 17.4 |

## Consequences for the design

- **The article context must stay small.** At about 207 t/s, 1,800 tokens of article take about 9 s before the first word. Budget: **≤ 1,800 tokens** of article per request (≈ 1,300 words), chosen as in P2 §11.5.
- The **chat panel** sends the B1 summary + relevant paragraphs (≈ 900 tokens) → about 5 s for the first answer. Follow-ups reuse the prompt cache (under 1 s to the first token) as long as no other request used the single slot in between.
- **B1 summary:** the first time takes about 9 s of reading + about 15 s of writing. It is cached afterwards. The UI shows "Reading the article…" with a progress hint.
- SPEC §16's "first text ≤ 4 s" for a summary cannot be met on an M1 with a full article. The new budget is: **cached: instant; first time: first text ≤ 12 s, complete ≤ 30 s**.
