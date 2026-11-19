# Web pages (browser_*)

For web applications use the **Web** actions — they drive Edge (default) or Chrome through the
DevTools protocol, address elements by their HTML **`element_id`** (or a CSS `selector`), work
without moving the mouse, and are far more reliable than clicking screen coordinates:

```json
{ "id": "open",  "action": "browser_open", "url": "https://example.com/contact", "browser": "edge" },
{ "id": "name",  "action": "browser_type", "element_id": "name", "text": "${person}" },
{ "id": "send",  "action": "browser_click", "element_id": "submit" },
{ "id": "wait",  "action": "browser_wait", "element_id": "confirmation", "timeout_ms": 15000 },
{ "id": "read",  "action": "browser_get_text", "element_id": "confirmation", "output_var": "msg" },
{ "id": "shot",  "action": "browser_screenshot", "path": "${flow_dir}\\output\\result.png" },
{ "id": "close", "action": "browser_close" }
```

- The field is `element_id` (bare id, no `#`) — every step already has its own `id`. Use
  `selector` for anything else (`form button[type=submit]`, `.result`, `table tr:nth-child(2) td`).
- `browser_open` accepts a URL or a local `.html` path; `headless: true` hides the window,
  `keep_open: true` leaves it open after the run. Only one browser per flow; the browser is
  closed automatically when the flow ends.
- `browser_type` clears the field first (`clear`) and can `press_enter`. `browser_get_text`
  reads visible text, `attribute: "value"` (inputs), `"html"`, or any attribute (`href`).
- `browser_wait` for the element that proves the next page/state arrived before reading it;
  for results that load in place (AJAX), `browser_wait_text` waits until some text (e.g.
  "Legal Name") appears, optionally inside one element, and can return the text.
- `browser_type` fires key/change events after inserting text, so Angular/React pages react
  (e.g. a CAPTCHA that only appears after a value is typed).
- `browser_screenshot` with `full_page: true` saves the whole page, not just the visible part
  (also for apps that scroll inside a container).
- **Finding element ids:** run `autoflow web-pick <url>` (or Studio's **🎯 Pick on page** on a
  web step): a browser opens, the user hovers (outline shows) and **Ctrl+clicks** the element;
  you get its `element_id`, or a `selector` when it has no id. No developer tools needed.
- **CAPTCHAs are always solved by the user, never by the flow or you.** Pattern: screenshot the
  CAPTCHA element, show it in `input_box` with `image`, type the user's answer:
  ```json
  { "id": "shot", "action": "browser_screenshot", "element_id": "imgCaptcha", "path": "${flow_dir}\\captcha.png" },
  { "id": "ask",  "action": "input_box", "title": "CAPTCHA", "prompt": "Type the characters shown", "image": "${flow_dir}\\captcha.png", "output_var": "captcha" },
  { "id": "fill", "action": "browser_type", "element_id": "fo-captcha", "text": "${captcha}" }
  ```
  Such flows need a person present: don't schedule them.
- `<select>` and anything else unusual: `browser_eval` runs JavaScript and returns its value,
  e.g. `"document.getElementById('topic').value = 'Support'"` or `"document.title"`.
- `browser_screenshot` saves the visible page, or one element with `element_id`/`selector`.
- The browser uses a private temporary profile (not the user's logged-in profile): sites that
  need a login must be logged into within the flow — ask the user for credentials via
  `input_box` (`password: true`) at run time, never hard-code them.
- To find ids, the user can right-click the element → Inspect in Edge/Chrome.
