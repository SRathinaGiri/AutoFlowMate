# Reading text on screen (OCR)

`ocr_read`, `ocr_find`, `ocr_click` use the OCR engine built into Windows (offline, no install).
Source: `window` (title part), `region` {x, y, width, height}, `monitor`, or `image` (a picture file;
not for `ocr_click`). `scale` 2 (default) helps with small fonts; `language` like `en-US` (the PC's
Windows languages; `autoflow ocr --languages` lists them).

- `ocr_read` → `output_var` text (lines), `lines_var`, `words_var` (each word with screen x/y).
- `ocr_find` → `found_var` / `x_var` / `y_var` for some `text` (may be several words); waits up to
  `timeout_ms`; never fails.
- `ocr_click` clicks the text (`offset_x`/`offset_y` to click the box beside a label), e.g. menu
  items in Tally or a remote desktop where UI Automation sees nothing.
- Test first: `autoflow ocr --window "Tally" --find "Balance Sheet"`. OCR confuses look-alikes
  (`1`/`I`, `0`/`O`): match on words, and don't use OCR output as exact codes without checking.
