use std::path::{Path, PathBuf};

use docx_rs::{BreakType, Docx, Paragraph, Run};
use pdfium_render::prelude::*;

use crate::error::ConversionError;
use crate::text_layout::{extract_paragraphs, TextChar};

/// Name of the Pdfium shared library on the current platform.
fn library_file_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "pdfium.dll"
    }
    #[cfg(target_os = "macos")]
    {
        "libpdfium.dylib"
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        "libpdfium.so"
    }
}

fn contains_library(dir: &Path) -> bool {
    dir.join(library_file_name()).is_file()
}

/// Accepts either the directory containing the Pdfium library, its parent
/// (`bin/` and `lib/` are checked), or the library file itself.
fn normalize_library_dir(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return path.parent().map(Path::to_path_buf);
    }

    let candidates = [path.to_path_buf(), path.join("bin"), path.join("lib")];
    candidates
        .into_iter()
        .find(|candidate| contains_library(candidate))
}

/// Candidate locations for a bundled Pdfium library, in priority order.
fn bundled_locations() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join("vendor").join("pdfium"));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.to_path_buf());

            // Walk up from `target/debug` (or `target/release`) to the project
            // root, where `vendor/pdfium` normally lives.
            for ancestor in dir.ancestors().skip(1).take(4) {
                candidates.push(ancestor.join("vendor").join("pdfium"));
            }
        }
    }

    candidates
}

/// Resolves the directory holding the Pdfium dynamic library.
///
/// Resolution order: explicit option, `PDFIUM_PATH`, the working directory's
/// `vendor/pdfium`, the executable's directory, and project-root ancestors.
/// Returns `None` if no bundled copy is found; callers can still fall back to a
/// system-installed library.
pub fn find_pdfium_library_dir(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        if let Some(dir) = normalize_library_dir(path) {
            return Some(dir);
        }
    }

    if let Some(path) = std::env::var_os("PDFIUM_PATH") {
        if let Some(dir) = normalize_library_dir(Path::new(&path)) {
            return Some(dir);
        }
    }

    for candidate in bundled_locations() {
        if let Some(dir) = normalize_library_dir(&candidate) {
            return Some(dir);
        }
    }

    None
}

pub(crate) fn bind_pdfium(explicit: Option<&Path>) -> Result<Pdfium, ConversionError> {
    // Pdfium's bindings live in a process-wide cell: only the first call may
    // load the library, and every later call must reuse those bindings. The
    // server converts many files per process, so this is the common path.
    if let Some(dir) = find_pdfium_library_dir(explicit) {
        let library = Pdfium::pdfium_platform_library_name_at_path(&dir);
        match Pdfium::bind_to_library(library) {
            Ok(bindings) => Ok(Pdfium::new(bindings)),
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
            Err(error) => Err(ConversionError::PdfiumLoad(format!(
                "{} ({error})",
                dir.display()
            ))),
        }
    } else {
        match Pdfium::bind_to_system_library() {
            Ok(bindings) => Ok(Pdfium::new(bindings)),
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
            Err(_) => Err(ConversionError::PdfiumNotFound),
        }
    }
}

/// Extracts text from a PDF and writes it as a `.docx` file.
///
/// One DOCX paragraph is emitted per detected PDF paragraph; PDF pages are
/// separated by page breaks. Images, tables and columns are not reconstructed
/// yet, which is the main fidelity limitation of this direction.
pub fn pdf_to_docx(
    input: &Path,
    output: &Path,
    pdfium_dir: Option<&Path>,
) -> Result<(), ConversionError> {
    if !input.is_file() {
        return Err(ConversionError::MissingInput(input.to_path_buf()));
    }

    let pdfium = bind_pdfium(pdfium_dir)?;
    let document = pdfium
        .load_pdf_from_file(input, None)
        .map_err(|error| ConversionError::Pdf(error.to_string()))?;

    let mut docx = Docx::new();
    let mut wrote_page = false;

    for page in document.pages().iter() {
        let text = page
            .text()
            .map_err(|error| ConversionError::Pdf(error.to_string()))?;

        let mut chars = Vec::new();

        for character in text.chars().iter() {
            let Some(ch) = character.unicode_char() else {
                continue;
            };

            if ch == '\r' || ch == '\n' || ch == '\u{0000}' {
                continue;
            }
            if ch.is_control() && ch != '\t' {
                continue;
            }

            let Ok(bounds) = character.tight_bounds() else {
                continue;
            };

            let size = character.scaled_font_size().value.max(1.0);

            chars.push(TextChar {
                ch,
                left: bounds.left().value,
                right: bounds.right().value,
                top: bounds.top().value,
                bottom: bounds.bottom().value,
                size,
            });
        }

        let paragraphs = extract_paragraphs(chars);
        if paragraphs.is_empty() {
            continue;
        }

        if wrote_page {
            docx =
                docx.add_paragraph(Paragraph::new().add_run(Run::new().add_break(BreakType::Page)));
        }
        wrote_page = true;

        for paragraph in paragraphs {
            docx = docx.add_paragraph(Paragraph::new().add_run(Run::new().add_text(paragraph)));
        }
    }

    if !wrote_page {
        docx = docx.add_paragraph(Paragraph::new());
    }

    let file = std::fs::File::create(output)?;
    docx.build()
        .pack(file)
        .map_err(|error| ConversionError::Docx(error.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_name_is_platform_specific() {
        let name = library_file_name();
        assert!(name.contains("pdfium"));
    }

    #[test]
    fn missing_input_is_reported() {
        let result = pdf_to_docx(
            Path::new("definitely-not-here.pdf"),
            Path::new("out.docx"),
            None,
        );
        assert!(matches!(result, Err(ConversionError::MissingInput(_))));
    }
}
