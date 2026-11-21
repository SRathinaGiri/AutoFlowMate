# AutoFlowMate agent plugin

Teaches an AI assistant to write, explain, fix, validate and run
[AutoFlowMate](https://github.com/SRathinaGiri/AutoFlowMate) flows: free, open-source Windows
automation (RPA) with plain JSON flows.

- Works without AutoFlowMate installed: the assistant writes the flow and says it is unvalidated.
- With AutoFlowMate on the user's Windows PC (`autoflow --version` works), the assistant checks
  flows with `autoflow validate`, tries them with `--dry-run`, and runs them after the user
  confirms anything that deletes, sends, submits or changes data.
- Never puts passwords in flows, never bypasses CAPTCHAs, never claims a run it didn't do.

Install AutoFlowMate: https://github.com/SRathinaGiri/AutoFlowMate/releases/latest

Privacy: the plugin and AutoFlowMate collect nothing; see
https://github.com/SRathinaGiri/AutoFlowMate/blob/main/PRIVACY.md. Licence: MIT.
