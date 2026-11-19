# E-mail: sending and reading

## Sending e-mail

Two actions. **Never** put a password in a flow; never ask the user to paste one into chat.

- **`outlook_send`**: uses the classic Outlook desktop app and the account already signed in,
  so no password is needed. `send_now: false` (default) opens the mail as a draft for the user
  to check and send; `send_now: true` sends at once (use this for scheduled flows). `account`
  picks a sender account by address. The new Outlook app can't be automated.
- **`send_email`**: SMTP with `provider` `gmail` / `outlook365` / `outlook.com` / `yahoo` /
  `zoho` (server and port filled in) or `custom` (+ `smtp_host`, `smtp_port`, `security`
  starttls|tls|none). `username` is the login (the e-mail address). `password_secret` is the
  *name* of a password stored in Windows Credential Manager: the user runs
  `autoflow secret set gmail` once (or uses 🔑 Set password… in Studio). **Gmail needs an App
  Password** (Google Account → Security → 2-Step Verification → App passwords). Microsoft has
  been turning off password logins for SMTP, so for Outlook/Microsoft 365 accounts prefer
  `outlook_send`.

Both take `to` / `cc` / `bcc` (separated by `;` or `,`), `subject`, `body` (`html: true` for
HTML) and `attachments`, a list of paths that can contain variables. A missing attachment fails
the step before anything is sent.

```json
{ "id": "mail", "action": "outlook_send", "to": "accounts@company.com",
  "subject": "Balance Sheet ${now:dd-MM-yyyy}", "body": "Please find attached.",
  "attachments": ["${export_path}"], "send_now": true }
```

Sending mail on the user's behalf is an outward action. Confirm recipients with the user, and
use `send_now: false` or a test address the first time.

## Reading e-mail

- **`read_email`** (IMAP): `provider` gmail / outlook365 / yahoo / zoho / zoho.in, or `imap_host`
  + `imap_port`; `username`; `password_secret` (Gmail: enable IMAP and use an App Password).
  Microsoft 365 mailboxes usually block password logins: use `outlook_read` for them.
- **`outlook_read`**: the classic Outlook desktop app, no password; `account` picks a mailbox;
  `folder` like `Inbox` or `Inbox/Invoices`.

Both filter with `unread_only` (default true), `from`, `subject_contains`, `since` (a date),
`max`, `newest_first`; afterwards `mark_read` and/or `move_to` a folder. `save_attachments_to` a
folder (existing files are not overwritten: `name (2).pdf`) with `attachment_filter` like `*.pdf`.
`output_var` gets a list of records `{from, from_name, to, subject, date, body, attachments
(saved paths), attachment_names}`; loop over them, then over `${mail.attachments}`. Reading
doesn't mark mail as read unless `mark_read` is set.
