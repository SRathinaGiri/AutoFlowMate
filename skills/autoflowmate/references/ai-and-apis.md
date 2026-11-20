# AI models and web APIs

`ai_prompt` sends one prompt and stores the reply. Providers:

| `provider` | Needs | Notes |
|---|---|---|
| `lmstudio` (default) | LM Studio's local server running with a model loaded (Developer tab → Start Server, port 1234) | Data stays on the PC; `model` empty = the loaded model |
| `ollama` | Ollama running (port 11434) and `model` | local |
| `openai_compatible` | `base_url` (e.g. `http://localhost:8000/v1`) | vLLM, LocalAI, Groq, OpenRouter… |
| `openai` | `api_key_secret` | default model `gpt-4.1-mini` |
| `gemini` | `api_key_secret` | default model `gemini-2.5-flash` |
| `anthropic` | `api_key_secret` | Claude; default model `claude-opus-5-5` |

These are **provider presets**. The user can change them or add more (Groq, OpenRouter, a company
server) in Studio under **File > Settings > AI providers**; they are saved in
`%APPDATA%\AutoFlowMate\presets.json`. A step's empty `model`, `base_url` and `api_key_secret`
take the preset's default model, server and key name (`openai` → secret `openai`, and so on), so
a minimal step is just `provider` + `prompt`. A flow that names a self-made preset needs the same
preset on the PC that runs it; for flows you hand to other people prefer the built-in names or
spell out `base_url` and `model`. In Studio, the ⟳ next to Model lists the models LM Studio or
Ollama currently has.

- `system` = instructions, `prompt` = the text with `${variables}`.
- **`labels`** (e.g. `["Fav", "UnFav", "Neutral"]`) tells the model to answer with one label and
  guarantees the stored value is exactly one of them; an answer matching none fails the step
  (add `retries`, and `on_error: continue` for batches).
- `json: true` asks for JSON and stores the parsed object (`${reply.sentiment}`).
- `temperature` (0 = predictable; not sent to Claude), `max_tokens` (default 4096; thinking models
  need room), `timeout_ms`.
- API keys: the user runs `autoflow secret set openai` once (or 🔑 Set key… in Studio) and the flow
  says `"api_key_secret": "openai"`. Never put a key in the flow.
- **Privacy:** cloud providers receive the text. Before sending customer or financial data to
  OpenAI/Gemini/Claude, confirm with the user; suggest `lmstudio` to keep data on the PC.
- For a batch, check once that a local server is up (see `classify_comments.json`: an
  `http_request` to `http://localhost:1234/v1/models` with `status_var`, then `stop` with a clear
  message) instead of failing every row.

## Web APIs (http_request)

`http_request`: `method`, `url`, `headers` (object), `body` (a string is sent as-is; an object is
sent as JSON), `bearer_secret` (token from Credential Manager → `Authorization: Bearer …`),
`save_to` (download a file), `output_var` (JSON responses become objects: `${response.items.0.name}`),
`status_var`, `fail_on_error_status` (default true), `timeout_ms`. In a dry run only GET/HEAD
requests that don't save a file are made.
