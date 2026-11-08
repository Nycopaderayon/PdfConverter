//! Conversion engine for pdfword.
//!
//! Four directions are supported:
//!
//! - `PDF -> Word`: text is extracted with Pdfium, grouped into lines and
//!   paragraphs, then written as a `.docx` file with `docx-rs`. An exact-look
//!   mode rasterizes pages instead.
//! - `WPS -> Word`: delegated to LibreOffice, which reads WPS Office and
//!   Microsoft Works (`libwps`) documents.
//! - `Word -> PDF`: delegated to a local LibreOffice installation, which gives
//!   far better layout fidelity than a hand-written renderer.
//! - `WPS -> PDF`: delegated to LibreOffice with the same font-embedding PDF
//!   export filter as `Word -> PDF`.

mod error;
mod libreoffice;
mod pdf_to_docx;
mod pdf_to_docx_visual;
mod text_layout;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use error::ConversionError;
pub use libreoffice::{docx_to_pdf, find_soffice, wps_to_docx, wps_to_pdf};
pub use pdf_to_docx::{find_pdfium_library_dir, pdf_to_docx};
pub use pdf_to_docx_visual::pdf_to_docx_exact;

/// Default rendering resolution for the exact-look mode.
pub const DEFAULT_EXACT_LOOK_DPI: f32 = 200.0;

/// The requested output format of a conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFormat {
    /// Convert a PDF into an editable Word document.
    Docx,
    /// Convert a Word document into a PDF.
    Pdf,
}

impl TargetFormat {
    pub fn extension(self) -> &'static str {
        match self {
            TargetFormat::Docx => "docx",
            TargetFormat::Pdf => "pdf",
        }
    }

    pub fn source_extension(self) -> &'static str {
        match self {
            TargetFormat::Docx => "pdf",
            TargetFormat::Pdf => "docx",
        }
    }
}

/// How a PDF is turned into a Word document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fidelity {
    /// Rebuild the document from extracted text. Fully editable, but layout,
    /// images and complex formatting are approximate.
    #[default]
    Editable,
    /// Render every PDF page to an image and place it on a same-sized Word
    /// page. The visual result matches the PDF closely, but it is not editable.
    ExactLook,
}

/// Optional configuration for the conversion engine.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// Explicit path to the LibreOffice executable (otherwise `SOFFICE_PATH` or common paths).
    pub soffice_path: Option<PathBuf>,
    /// Directory containing the Pdfium shared library (otherwise `PDFIUM_PATH`).
    pub pdfium_dir: Option<PathBuf>,
    /// Maximum time a single LibreOffice conversion may run.
    pub timeout: Option<Duration>,
    /// Output strategy for PDF -> Word conversions.
    pub fidelity: Fidelity,
    /// Resolution used to rasterize pages in [`Fidelity::ExactLook`].
    pub exact_look_dpi: f32,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            soffice_path: None,
            pdfium_dir: None,
            timeout: None,
            fidelity: Fidelity::Editable,
            exact_look_dpi: DEFAULT_EXACT_LOOK_DPI,
        }
    }
}

/// Converts `input` into `output`, writing the result to exactly `output`.
pub fn convert(
    input: &Path,
    output: &Path,
    target: TargetFormat,
    options: &ConvertOptions,
) -> Result<(), ConversionError> {
    let is_wps = matches!(
        input
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("wps" | "wpt")
    );

    match target {
        TargetFormat::Docx => {
            if is_wps {
                let out_dir = output.parent().unwrap_or_else(|| Path::new("."));
                let timeout = options.timeout.unwrap_or(Duration::from_secs(120));
                let produced =
                    wps_to_docx(input, out_dir, options.soffice_path.as_deref(), timeout)?;
                if produced != output {
                    std::fs::rename(&produced, output)?;
                }
                return Ok(());
            }

            match options.fidelity {
                Fidelity::Editable => pdf_to_docx(input, output, options.pdfium_dir.as_deref()),
                Fidelity::ExactLook => pdf_to_docx_exact(
                    input,
                    output,
                    options.pdfium_dir.as_deref(),
                    options.exact_look_dpi,
                ),
            }
        }
        TargetFormat::Pdf => {
            let out_dir = output.parent().unwrap_or_else(|| Path::new("."));
            let timeout = options.timeout.unwrap_or(Duration::from_secs(120));
            let soffice = options.soffice_path.as_deref();
            let produced = if is_wps {
                wps_to_pdf(input, out_dir, soffice, timeout)?
            } else {
                docx_to_pdf(input, out_dir, soffice, timeout)?
            };
            if produced != output {
                std::fs::rename(&produced, output)?;
            }
            Ok(())
        }
    }
}
