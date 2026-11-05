# Padd

A light, fast desktop editor for **Markdown** and **structured text** (JSON, YAML, XML, code, logs…)
with **encrypted secrets files**. Linux, Windows and macOS: one ~9 MB native binary, no webview.

- **Tabs**: open several files; drag and drop works; `padd file1 file2` opens files from the command line.
- **Live Markdown preview while you edit**: headings, bold, italic, code, links, lists, quotes and
  tables render in place. The line under the cursor shows its raw Markdown. `Ctrl+E` toggles this.
- **Syntax highlighting** for structured text and code, re-highlighting only the lines that changed.
- **Text only**: binary files are refused. UTF-8, UTF-8 BOM and UTF-16 are supported, and CRLF/LF is kept on save.
- **Large files**: files over 4 MB open in a fast read-only viewer.

## Secrets

Write any line as label::: value

```markdown
## OpenAI
url: https://platform.openai.com
api key::: sk-proj-abc123
```

When the cursor is elsewhere, the value is masked as `••••••••` with **show** and **copy**
buttons. Copying clears the clipboard after 30 s. Lines that *start* with `:::`
(Pandoc/Docusaurus directives) are left alone.

## Encryption

*File ▸ Encrypt with password…* (or *New secrets file…*) saves the document as `name.md.enc`:

- The format is [age](https://age-encryption.org) with a passphrase (scrypt) and ASCII armor, so the
  file is still plain text and can be decrypted without Padd: `age -d keys.md.enc`.
- One password per file. Opening an encrypted file shows an unlock field in its tab.
- Plaintext lives only in memory. Saves are atomic and write only ciphertext.
  `Ctrl+L` locks a tab, and saved tabs auto-lock after 10 minutes without input.
- There is no password recovery.

## Shortcuts

| Keys | Action |
|---|---|
| Ctrl+N / Ctrl+O | new / open |
| Ctrl+S / Ctrl+Shift+S | save / save as |
| Ctrl+W | close tab |
| Ctrl+Tab / Ctrl+Shift+Tab | next / previous tab |
| Ctrl+E | toggle live preview |
| Ctrl+L | lock encrypted tab |

(Cmd instead of Ctrl on macOS.)

## Build

```sh
cargo build --release          # target/release/padd
cargo test
```

CI runs on the Mac mini: `ci/ci.sh` (macOS universal `.app`, Linux arm64/x86_64, Windows x86_64 → `dist/`).
Design notes and the research behind the choices are in [docs/DESIGN.md](docs/DESIGN.md).

## License

[MIT](LICENSE)
