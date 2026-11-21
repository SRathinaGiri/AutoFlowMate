# Privacy policy

*Applies to AutoFlowMate (the programs `autoflow.exe` and `autoflow-studio.exe`) and to the
AutoFlowMate agent skill / plugin for AI assistants.*

## In short

AutoFlowMate collects nothing. It has no account, no telemetry, no analytics and no server. It
runs on your Windows PC, and your flows, files and results stay there.

## The programs

- **No data is sent to the developer** or to anyone else by AutoFlowMate itself. There is no usage
  tracking, crash reporting or update check.
- **Your settings** (Studio preferences, AI and e-mail provider lists, your own templates) are
  stored in `%APPDATA%\AutoFlowMate` on your PC.
- **Passwords and keys** you store with `autoflow secret set` (or the 🔑 button) are kept in the
  Windows Credential Manager of your Windows account, never in flow files or logs.
- **Redaction and pseudonymisation** run entirely on your PC. The pseudonymisation vault is a file
  you choose, encrypted with a key kept in your Credential Manager.
- **Data leaves your PC only when a flow you run tells it to:** for example a step that sends an
  e-mail, calls a web API, opens a web page, or asks a cloud AI provider (OpenAI, Google Gemini,
  Anthropic Claude) a question. That data then goes directly from your PC to the service you
  chose, under that service's own privacy policy. Local AI (LM Studio, Ollama) stays on your PC.

## The agent skill / plugin

The skill is a set of instructions and reference documents that teach an AI assistant (such as
Claude or ChatGPT) to write AutoFlowMate flows. It contains no code that runs on its own and
collects nothing. What you type to your AI assistant, and the flows it writes, are handled by
that assistant's provider under its privacy policy. When the assistant runs `autoflow` on your
PC, the programs behave as described above.

## Contact

Questions or concerns: open an issue at https://github.com/SRathinaGiri/AutoFlowMate/issues.
Security problems: please report them privately as described in
[SECURITY.md](SECURITY.md).
