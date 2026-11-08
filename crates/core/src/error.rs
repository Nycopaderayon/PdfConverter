use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ConversionError {
    #[error("input file not found: {0}")]
    MissingInput(PathBuf),

    #[error("unsupported conversion: {0}")]
    Unsupported(String),

    #[error("LibreOffice was not found; install it or set SOFFICE_PATH")]
    LibreOfficeNotFound,

    #[error(
        "Pdfium shared library was not found; set PDFIUM_PATH or run the provided fetch script"
    )]
    PdfiumNotFound,

    #[error("failed to load Pdfium: {0}")]
    PdfiumLoad(String),

    #[error("conversion timed out after {0:?}")]
    Timeout(Duration),

    #[error("external process failed: {0}")]
    Process(String),

    #[error("PDF error: {0}")]
    Pdf(String),

    #[error("failed to render a PDF page: {0}")]
    Render(String),

    #[error("Word document error: {0}")]
    Docx(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
