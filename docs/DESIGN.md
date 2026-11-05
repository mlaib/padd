# Padd — design notes

Padd is a small, fast desktop editor for Markdown and structured text
(Linux, Windows, macOS). It can also keep passwords and API keys in
encrypted files.

## Goals
1. **Light**: one small native binary, no webview, low RAM, near-zero idle CPU.
2. **Fast**: opens instantly, and typing never waits on layout or crypto.
3. **Simple**: a secrets file is plain Markdown plus one rule (`:::`), and any text file can be encrypted.

## Research findings (Nov 2026) and the decisions they drove

| Topic | Finding | Decision |
|---|---|---|
| GUI toolkit | egui/eframe 0.37 (Oct 2026) is mature and reactive. It repaints only on input, so idle CPU is ~0. Slint is comparable, iced is heavier for text editing, Tauri needs a webview. | **eframe 0.37** |
| Renderer | Measured on a stripped release binary: eframe+glow = 4.7 MB, eframe+wgpu = 8.0 MB | **glow** (OpenGL) |
| Bundled fonts | `default_fonts` adds about 0.65 MB. `system_fonts` adds about 0.5 MB and looks different on every OS. | Keep `default_fonts` |
| Clipboard | eframe `clipboard` adds about 0.65 MB, but copy/paste is essential | Use eframe's clipboard (no separate `arboard`) |
| Accessibility | `accesskit` adds about 1.6 MB | Off by default (`--features a11y`) |
| Encryption | `age` + armor adds about 74 KB. A hand-rolled Argon2 + XChaCha20 scheme adds 44 KB. | **age**: audited, a standard format, and decryptable by the `age` CLI without Padd. Armored, so the file stays text. |
| Markdown | `pulldown-cmark` adds about 0.3 MB. Its offset iterator gives byte ranges for live styling. | pulldown-cmark |
| Highlighting | `syntect` (pure-Rust regex, `default-fancy`) adds about 0.4 MB. tree-sitter bundles are many MB. syntect's `ParseState` and `HighlightState` are `Eq`, which makes incremental re-highlighting possible. | syntect, **loaded lazily on a background thread**, incremental line cache |
| Large files | Ferrite (an egui Markdown editor) found that a whole-buffer `TextEdit` on a 4 MB file used about 1.8 GB. Its fix was a rope buffer, virtual scrolling and a per-line galley cache. | v1: `TextEdit` up to 4 MB; above that, a virtualised read-only viewer. v2: custom rope editor. |
| Binary size | `opt-level="s"`, fat LTO, `codegen-units=1`, `panic="abort"`, `strip`. Measured `s` vs `z`. | Release profile below |

| System fonts | eframe `system_fonts` loads OS fallback fonts on demand (fontique: CoreText, DirectWrite, fontconfig) for emoji, CJK and Arabic | Enabled (+0.5 MB). UI icons are vector-drawn, so they never depend on a font. |
| Native libraries | X11, Wayland, EGL/GLX, fontconfig and xkbcommon are all `dlopen`ed | Linux binary links only libc, libm and libgcc, so cross-compiling needs only a linker |
| KDF speed | scrypt 2^18 took 1.9 s at `opt-level="s"` and 1.3 s with the crypto crates at `opt-level=3` | Per-package `opt-level=3` for scrypt, salsa20, sha2 and chacha |

**Measured (Linux x86_64):**
- Release binary: **9.1 MB** (stripped, everything included).
- RSS: about 115–125 MB under Xvfb with software OpenGL (llvmpipe). With a real GPU driver it is lower.
- Idle CPU is ~0: egui repaints only on input or a timer.

## Live preview (Markdown)
The same `TextEdit` is used for editing and rendering. A custom layouter turns the
pulldown-cmark spans into a `LayoutJob`:
- Headings are larger, emphasis is italic, code is monospace on a background, and links are coloured.
- Syntax markers (`#`, `**`, `` ` ``, `[](url)`) are drawn transparent at 1 px size, *except on
  the line(s) holding the cursor*. Off the active line you see rendered text; on it you see the
  source. Character indices never change, so the cursor and selection stay correct.
- Tables are drawn in monospace so the columns line up.

## Secrets syntax
```
## OpenAI
url: https://platform.openai.com
api key::: sk-abc123
```
- `label::: value`: when the cursor is not on that line, the value is hidden and shown as
  `••••••••` with **[show]** and **[copy]** buttons. The space for the buttons comes from
  `LayoutSection::leading_space`, so the text itself is untouched.
- Lines that *start* with `:::` (Pandoc/Docusaurus directives) and fenced code are ignored.

## Encryption
- `name.md.enc` (any `name.ext.enc`) is an ASCII-armored age file using a scrypt passphrase
  (work factor 2^18, fixed so it decrypts the same on slow and fast machines).
- One password per file. On open, the tab shows an inline unlock field. Decryption and
  encryption run on a worker thread, so the UI never freezes.
- Plaintext lives only in memory. Saves are atomic (temp file + rename) and only ever write
  ciphertext. On lock or close, the text and the egui undo state are wiped (`zeroize`).
- Copying a secret clears the clipboard after 30 s. Unlocked tabs with no unsaved changes
  auto-lock after 10 minutes without input.
- Known limit: egui's galley caches can hold text fragments until they are evicted (a few frames).

## Text-only policy
Padd opens a file only if it decodes as UTF-8, UTF-8 with BOM, or UTF-16 with a BOM, and has
no NUL bytes in the first 8 KB. Otherwise it refuses ("binary file"). CRLF and BOM are kept on save.

## Build / CI (Mac mini)
- `ci/ci.sh` copies the source tree to the mini and runs:
  - macOS (native): fmt check, clippy, tests, then a universal `Padd.app` (arm64 + x86_64) as a zip.
  - Docker on the mini (`ci/Dockerfile`, Debian bookworm, so glibc 2.36): Linux aarch64,
    Linux x86_64 (cross gcc) and Windows x86_64 (mingw).
- Artifacts are copied back to `dist/`.

## Roadmap
1. ✅ Tabs, open/save, text-only check, highlighting, Markdown live preview, secrets, age encryption
2. Rope-based custom editor for very large files; syntax-highlighted code fences in Markdown
3. Inline images and table rendering; find/replace; recent files; settings file
4. Signed installers (`.deb`/AppImage, `.msi`, notarised `.dmg`)
