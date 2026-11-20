# Contributing to AutoFlowMate

Thank you for helping. AutoFlowMate is a free, MIT-licensed Windows desktop automation tool, and
contributions of every size are welcome: bug reports, ideas, translations, example flows,
documentation and code.

By contributing you agree that your work is published under the project's [MIT licence](LICENSE).
Please be kind and constructive; see the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to help without code

- **Report a bug** or **suggest a feature** with the issue forms (Issues > New issue). Include the
  flow JSON if you can, with any personal data removed.
- **Translate Studio:** copy `lang/template.json` to `lang/<language-code>.json` (for example
  `hi.json`), set `"_language"` to the language's name and fill in the translations. Untranslated
  texts can stay empty; they show in English.
- **Share an example flow** for a common task. Examples must be safe to run: they work only inside
  `examples/` (or open their own windows), and never need anyone's real account.

## How a code change gets in

1. **Open an issue first** for anything bigger than a small fix, so we can agree on the approach
   before you spend time on it.
2. **Fork** the repository and create a branch from `main` (`fix-ocr-timeout`, `add-ftp-upload`...).
3. Make the change, with tests (see below).
4. Run the same checks CI runs:

   ```powershell
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

   If you added or changed an action, regenerate the schema and the action reference and commit
   them:

   ```powershell
   cargo run -p autoflow-cli -- schema -o schema/flow.schema.json
   cargo run -p autoflow-cli -- docs -o skills/autoflowmate/references/actions.md
   ```

5. Open a **pull request** to `main` and fill in the checklist in the template.

### What happens to your pull request

- **CI runs automatically** on Windows: formatting, Clippy with warnings as errors, build, all
  tests, validation of every example flow, a check that the generated schema and action reference
  are up to date, and a licence check of every dependency (`deny.toml`). A first-time
  contributor's CI run starts after the maintainer approves it.
- The `main` branch is protected: a pull request can be merged only when **CI passes** and the
  **maintainer has approved** it. Only the maintainer pushes to `main` directly.
- Pull requests are **squash-merged**, so each one becomes one commit with a clear message.
- The maintainer may ask for changes. If a pull request waits for its author for 30 days, it may
  be closed; it can always be reopened.

## Rules for code

### Safety first

AutoFlowMate moves the mouse, types, sends mail and touches files on real PCs, so these rules are
not negotiable:

- **No secrets in flows.** Passwords and API keys live in the Windows Credential Manager
  (`autoflow secret set NAME`); a step refers to them by name. Never log a secret; events mask
  them.
- **Never solve or bypass CAPTCHAs** or other bot checks. Show them to the user to type.
- **Don't touch what the flow didn't create:** don't close or kill other windows or programs, and
  only clean up files inside the flow's own output folders.
- **The fail-safe must keep working:** mouse into a screen corner or Ctrl+Alt+Q stops any flow.
- **Dry runs never change anything:** every action that has effects must honour `--dry-run`.
- Sending mail through Outlook should default to opening a draft.

### Dependencies

- Only **permissive licences** (MIT, Apache-2.0, BSD, ISC, Zlib and similar, as listed in
  `deny.toml`). **No GPL, LGPL, AGPL, MPL or other copyleft** code or crates. CI blocks them.
  The only exception, approved by the maintainer, is `auto_generate_cdp`, a build-time code
  generator used by headless_chrome; none of its code ends up in the programs.
- Crates come from crates.io only. Prefer well-maintained, widely used crates, and explain a new
  dependency in the pull request.

### Style

- Rust stable, formatted with `rustfmt` (`rustfmt.toml`), no Clippy warnings.
- Windows only: the app targets Windows 10/11.
- Actions are declared once in the `actions!` macro (`crates/autoflow-core/src/actions/mod.rs`);
  the schema, docs and Studio forms come from it. Give every field a clear doc comment: it becomes
  the help text users see in Studio.
- Texts shown in Studio go through `tr("English text")` so they can be translated.
- Error messages should tell the user what to do next, in plain words.

### Tests

- Add unit tests for new logic, next to the code (`#[cfg(test)]`).
- Tests must run on a normal Windows PC **without moving the mouse, typing, opening windows,
  sending mail or using the network**. Anything that needs those belongs in an example flow, not
  in `cargo test`.
- Use unique temporary folders in tests so they can run in parallel.
- Add or update an example flow when you add a user-facing feature; CI validates every example.

## Releases

Only the maintainer releases. See "Releasing" in the [README](README.md#for-developers).

## Questions

Open a discussion or an issue. Thank you for making AutoFlowMate better!
