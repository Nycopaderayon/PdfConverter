# Nyc-pdfword Converter

Open-source, self-hosted **PDF ⇄ Word**, **WPS → Word** and **WPS → PDF** converter.

- **PDF → Word** — extracts positioned text with [Pdfium](https://pdfium.googlesource.com/pdfium/) and rebuilds paragraphs, page breaks, font size and basic spacing into a `.docx` with [docx-rs](https://github.com/bokuweb/docx-rs). An **Exact look** mode instead embeds each page as a high-resolution image for pixel-faithful output.
- **WPS → Word** — converts `.wps` documents and `.wpt` templates (WPS Office and Microsoft Works) to `.docx` with LibreOffice.
- **Word → PDF** — rendered by a local LibreOffice installation for high layout fidelity, with fonts embedded in the exported PDF.
- **WPS → PDF** — converts `.wps` / `.wpt` documents to PDF with the same LibreOffice font-embedding export.
- **Web app** — drag-and-drop UI, fidelity selector, asynchronous jobs, progress polling, automatic temp-file cleanup.
- **Single small Rust service** — no database, no accounts, no tracking.

## Why this design

PDF has no notion of paragraphs, tables or columns — only positioned glyphs. Reconstructing a Word document is therefore inherently heuristic. This project focuses on text fidelity first: characters are grouped into visual lines and paragraphs using their coordinates.

Rendering Word documents to PDF is a different problem: a full layout engine (fonts, line breaking, pagination, tables) is required. Rather than reimplementing it, pdfword delegates Word → PDF to LibreOffice headless, which is mature and reliable.

## Quick start (Docker)

```bash
docker build -t pdfword .
docker run --rm -p 8080:8080 pdfword
```

Open <http://localhost:8080>.

The image includes LibreOffice Writer and downloads the Pdfium shared library at build time.

To use documents that rely on fonts not shipped in the image, mount them into the
custom font directory. The entrypoint refreshes the font cache on startup:

```bash
docker run --rm -p 8080:8080 \
  -v "$PWD/fonts:/usr/share/fonts/truetype/custom:ro" \
  pdfword
```

Fonts must be installed (or mounted) **before** conversion: if LibreOffice cannot
find a font, it substitutes another one before the PDF is exported, so font
embedding cannot recover the original.

## Local development

Requirements:

1. **Rust** (stable, edition 2021).
2. **LibreOffice** — needed for Word → PDF, WPS → Word and WPS → PDF.
3. **Pdfium** — only needed for PDF → Word.

Fetch Pdfium into `vendor/pdfium`:

```powershell
# Windows (PowerShell)
.\scripts\fetch-pdfium.ps1
```

```bash
# Linux / macOS
./scripts/fetch-pdfium.sh
```

The server auto-detects `vendor/pdfium/bin` (Windows) or `vendor/pdfium/lib`
(Linux/macOS) relative to the project. Set `PDFIUM_PATH` only if you keep the
library somewhere else.

Point the server at LibreOffice if it is not on `PATH`:

```powershell
$env:SOFFICE_PATH = "C:\Program Files\LibreOffice\program\soffice.exe"
```

Run the server:

```bash
cargo run -p pdfword-web
# listening on http://127.0.0.1:8080
```

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `BIND_ADDR` | `127.0.0.1:8080` | Listen address (use `0.0.0.0:8080` in containers) |
| `WORK_DIR` | system temp dir | Root for temporary job directories |
| `MAX_UPLOAD_MB` | `25` | Maximum upload size |
| `MAX_CONCURRENT` | `2` | Maximum simultaneous conversions |
| `CONVERT_TIMEOUT_SECS` | `120` | LibreOffice conversion timeout |
| `JOB_TTL_SECS` | `3600` | How long finished outputs are kept |
| `SOFFICE_PATH` | auto-detect | LibreOffice executable |
| `PDFIUM_PATH` | auto-detect | Directory containing `pdfium.dll` / `libpdfium.so` (also checks `vendor/pdfium`) |
| `EXACT_LOOK_DPI` | `200` | Rendering resolution for the PDF → Word “Exact look” mode (72–600) |

## HTTP API

| Method | Path | Description |
| --- | --- | --- |
| `POST` | `/api/jobs` | `multipart/form-data` with `file`, `target`, and optional `uid`. `target=docx` accepts PDFs and WPS documents; `target=pdf` accepts Word and WPS documents. Optional `fidelity` (`editable` or `exact`) applies to PDF → Word only. Returns `{"id": "..."}`. |
| `GET` | `/api/jobs/{id}` | Returns `{id, status, input_name, output_name, error}`. `status` is `queued`, `processing`, `done` or `failed`. |
| `GET` | `/api/jobs/{id}/download` | Downloads the converted file. |
| `GET` | `/api/stats` | Returns real-time usage statistics `{"users": N, "conversions": M}`. |
| `GET` | `/healthz` | Returns `ok`. |

Example:

```bash
# Editable Word document
curl -F "file=@report.pdf" -F "target=docx" http://localhost:8080/api/jobs

# Exact-look Word document (page images)
curl -F "file=@report.pdf" -F "target=docx" -F "fidelity=exact" http://localhost:8080/api/jobs

# WPS Writer document to Word
curl -F "file=@report.wps" -F "target=docx" http://localhost:8080/api/jobs

# WPS document to PDF
curl -F "file=@report.wps" -F "target=pdf" http://localhost:8080/api/jobs

# Word document to PDF
curl -F "file=@report.docx" -F "target=pdf" http://localhost:8080/api/jobs
```

## Limitations

- PDF → Word does not reconstruct images, tables or multi-column layouts yet. Rotated text and unusual encodings may also be lossy.
- **Exact look** keeps the visual design by embedding each page as an image; the resulting Word file is not editable and is larger than an editable conversion. Pages with differing sizes are scaled to fit the first page.
- Scanned PDFs contain no text layer, so an editable conversion extracts nothing; use **Exact look** for those, or wait for OCR.
- Word → PDF depends on LibreOffice being installed in the runtime environment.
- WPS → Word / WPS → PDF rely on LibreOffice. The `.wps` extension is shared by two unrelated formats, both detected by content: WPS Office (Kingsoft) files, which use the Word 97 binary format, and Microsoft Works word-processor files (DOS 2.0 through Works 6.0+), read via `libwps`. Complex layouts, embedded objects and exotic fonts may lose fidelity. Some Microsoft Works Word Processor 3.0 files may show stray style labels (e.g. `ColorSet-1StyleSet-1`) at the top of the document; this comes from the LibreOffice/libwps import, not from pdfword.
- Uploaded files are deleted after `JOB_TTL_SECS`; jobs are kept in memory only.

## Security

Treat this as any file-processing endpoint:

- Put it behind a reverse proxy with authentication before exposing it to the internet.
- Run it in the provided container. Pdfium and LibreOffice both parse untrusted input.
- Upload size, concurrency and conversion time are bounded (`MAX_UPLOAD_MB`, `MAX_CONCURRENT`, `CONVERT_TIMEOUT_SECS`).
- Job ids are random UUIDs, and outputs plus uploads are deleted after `JOB_TTL_SECS`.
- Headless conversion does not execute document macros, but sandboxing is still recommended.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Layout heuristics live in `crates/core/src/text_layout.rs` and are covered by unit tests. `scripts/fetch-pdfium.*` helps set up an end-to-end environment.

## Roadmap

- [ ] Image extraction from PDF into Word
- [ ] Table detection
- [ ] OCR fallback for scanned PDFs (Tesseract)
- [ ] Column detection and reading-order reconstruction
- [ ] Optional pure-Rust Word → PDF backend
- [ ] CLI (`pdfword convert input.pdf output.docx`)

## License

MIT. Pdfium (BSD-3-Clause) and LibreOffice (MPL-2.0) are external dependencies and are not vendored in this repository.
