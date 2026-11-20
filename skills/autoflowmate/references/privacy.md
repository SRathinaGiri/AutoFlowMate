# Privacy: redaction and pseudonymisation (DPDP)

Everything here runs offline, without AI. Use it before documents or tables leave the
organisation (vendors, auditors, analysts, AI services).

## Redact documents: `redact_files`

Makes redacted **copies** of one file or a whole folder (sub-folders kept); the originals are
never changed and `output` must differ from `input`.

```json
{ "id": "redact", "action": "redact_files",
  "input": "C:\\Docs\\to_share", "output": "C:\\Docs\\redacted",
  "patterns": { "EMPLOYEE_ID": "EMP-\\d{5}" },
  "words_file": "names_to_redact.txt",
  "report": "C:\\Docs\\redacted\\report.csv",
  "summary_var": "redaction" }
```

- Input: one file (output = a file of the same type, or a folder) or a folder (sub-folders
  kept). `include` picks files by wildcards: `"*.pdf;*.docx"`; empty = every supported type.
- Formats: PDF (text and scanned), Word `.docx`, Excel `.xlsx`, pictures (PNG, JPG, BMP, TIFF),
  text, CSV, JSON, XML, HTML, Markdown, logs. Other files are skipped and **not copied**.
  Not supported: old `.doc`/`.xls`, `.pptx`, e-mail `.msg`, archives - convert them first.
- Word and Excel keep their formatting; text split over formatting runs is still found; numbers
  typed as numbers in Excel (mobiles, accounts) are found; pictures inside are OCR'd and blacked
  out; `clear_metadata` empties author fields (also comment and tracked-change authors) and redacts
  the document properties.
- PDFs and pictures become pictures with black boxes (no text layer, no metadata left). Text
  PDFs use their own character positions; scanned pages and pages with pictures are read with
  Windows OCR (`ocr`: auto / always / never; `pdf_dpi` default 150; use 200-300 for small print).
- **Fail closed.** If any file fails or can't be checked completely (`unverified`: a page or
  picture where OCR read nothing although it isn't blank, a picture format OCR can't open, or OCR
  off), that file is **not written** and the step **fails** after writing the report, so later
  steps (zip, mail, upload) never see an incomplete folder. Only set `continue_on_problems: true`
  when the user explicitly accepts that, and then check `summary.failed` / `summary.unverified`.
- `summary_var` gets `{files, redacted, clean, unchanged, unverified, skipped, failed, found: {PAN: n, ...}}`.
  `clean` means read completely and nothing found. `skipped` = file type not supported (not copied).
- **Existing outputs need proof.** The output folder keeps a manifest
  (`.autoflowmate-redaction.json`) with the SHA-256 of each input and output and a hash of the
  settings. A file already in the output folder is reused (`unchanged`) only when all three still
  match; otherwise it is redone. A file that fails is removed from the output folder, even if an
  earlier run had left it there. Outputs are written to a temporary file and renamed into place.
- Limits: files over 1 GB, PDFs over 5,000 pages, pictures over 150 megapixels and Office files
  that unpack to over 4 GB fail (and are not written).
- The report lists counts per file, never the values.
- Thousands of files: it uses several threads (`threads`, 0 = automatic). With `overwrite: false`
  (default) an interrupted run continues: files the manifest proves finished are not redone.

## Redact text: `redact_text`

`input` → `output_var` (cleaned text), `report_var` (counts). Same rules, patterns and words.
`mask_style`: `label` ([PAN]), `mask` (XXXXXXXXXX), `partial` (XXXXXX234F, last 4 kept),
`text` (`replace_with`). PDFs and pictures always get black boxes.

## Rules (`rules`; empty = all, `none` = only your patterns/words)

| rule | finds |
|---|---|
| pan | PAN `ABCPE1234F` (4th letter must be a valid holder type) |
| aadhaar | 12 digits, spaces optional, first digit 2-9, **Verhoeff checksum** (`strict`) |
| mobile | `98765 43210`, `+91 98765 43210`, `09876543210` |
| email, upi | e-mail addresses; UPI IDs `name@okaxis` |
| bank_account | digits after `A/c`, `A/c No`, `Account No`, `Acct` |
| ifsc | `SBIN0001234` |
| card | 13-19 digits with **Luhn checksum** |
| passport, voter_id, driving_licence | `K1234567`, `ABC1234567`, `TN01 20190012345` |
| dob, age, gender | after labels: `DOB`, `Date of Birth`, `Age:`, `Sex:`/`Gender:` |
| name | after `Name:`, `Patient Name:`, `Father's Name:`, `S/o`, `D/o`, `W/o`, or a title (`Mr`, `Mrs`, `Dr`, `Shri`, `Smt`...) |
| patient_id | after `UHID`, `MRN`, `MR No`, `IP No`, `OP No`, `Patient ID`, `Reg No` |
| health | after `Diagnosis:`, `Complaints:`, `Condition:` (to the end of the line) |
| address, pincode | after `Address:`, `R/o`; `PIN:` |

Labelled rules redact only the value: `Name: [NAME]`. Names **without** a label cannot be found
by patterns: put them in `words` or a `words_file` (one per line, or the first CSV column).

Own patterns (`patterns`): `{"LABEL": "regex"}`. Name a group `value` to redact only that part:
`{"POLICY": "Policy No:\\s*(?P<value>\\S+)"}`. Start with `(?i)` to ignore case. Rust regex syntax
(no look-around). Test with `redact_text` first.

## Pseudonymise: `pseudonymize` / `depseudonymize`

Replace IDs with tokens that can be turned back only with the key:

```json
{ "id": "acc", "action": "pseudonymize", "data": "${rows}", "columns": ["AccountNo"],
  "prefix": "ACC-", "domain": "account", "vault": "vault/customers.db", "output_var": "rows" }
{ "id": "back", "action": "depseudonymize", "data": "${shared}", "columns": ["AccountNo"],
  "vault": "vault/customers.db", "unknown": "fail", "output_var": "restored" }
```

- One value: `input` → `output_var`. A table: `data` (records from `read_csv`/`read_excel`) +
  `columns` → `output_var` (the whole table, other columns unchanged).
- Token = prefix + 16 characters of a keyed HMAC-SHA-256. Same value + domain → same token
  (joins and counts still work). Without the key nobody can compute or reverse a token, not even
  by trying every mobile number (a plain hash could be reversed that way).
- The vault (SQLite) stores each original encrypted with AES-256-GCM. Restoring needs the vault
  **and** the key. Wrong key → clear error.
- The key: Windows Credential Manager, secret `key_secret` (default `pseudonym_key`), created on
  first use. Tell the user to back it up: `autoflow secret show pseudonym_key`. To share between
  PCs: `autoflow secret set pseudonym_key` with the same key (or a long passphrase) on each.
  `autoflow secret new-key NAME` makes one up front.
- `normalize: true` removes spaces/dashes and uses capitals first (the vault keeps the tidied value).
- `unknown` in depseudonymize: `keep` (default), `empty`, or `fail` (safest for payouts).
- Dry run: tokens are computed but the vault is not written.

## Limits and threat model (tell the user when it matters)

- Pattern matching and OCR have **false negatives**: unlabelled names, free-text addresses,
  handwriting, rotated or very poor scans, other scripts (OCR uses the installed Windows languages)
  can be missed. Redacted output still needs a human spot-check before external release.
- These steps **support** DPDP-style privacy work; using them does not by itself make a process
  compliant.
- **Pseudonymised is not anonymous.** Tokens are deterministic: equal values stay equal (frequency
  and linkage are visible), and other columns (names, amounts, dates) can still identify people.
  Use a separate `domain` per purpose so different shares can't be joined.
- Whoever has the **vault and the key** can restore everything: keep the vault in a protected
  folder, back up the key separately (`autoflow secret show NAME`), and decide who may run
  `depseudonymize`. Losing the key means the tokens can never be turned back.

Examples: `examples/redact_documents.json`, `examples/pseudonymize_accounts.json`. Templates:
`templates/redact_folder.json`, `redact_on_arrival.json`, `pseudonymize_table.json`,
`restore_pseudonyms.json` (`autoflow new my.json --template redact_folder`).
