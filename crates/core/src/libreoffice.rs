use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use wait_timeout::ChildExt;

use crate::error::ConversionError;

/// LibreOffice PDF export filter with explicit font embedding.
///
/// JSON filter options are supported by LibreOffice 7.4 and newer (Debian
/// bookworm ships 7.4, the Windows installer is newer). Embedding standard
/// fonts keeps Word documents that rely on common fonts (Arial, Times New
/// Roman, ...) looking the same on machines without them.
const PDF_EXPORT_FILTER: &str = concat!(
    "pdf:writer_pdf_Export:",
    r#"{"EmbedStandardFonts":{"type":"boolean","value":"true"},"#,
    r#""EmbedOnlyUsedFonts":{"type":"boolean","value":"true"}}"#
);

/// Resolves the LibreOffice executable.
///
/// Resolution order: explicit option, `SOFFICE_PATH`, `PATH`, then the
/// conventional install locations for Windows, macOS and Linux.
pub fn find_soffice(explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        if path.is_file() {
            return Some(path.to_path_buf());
        }
    }

    if let Some(path) = std::env::var_os("SOFFICE_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }

    let executable = if cfg!(windows) {
        "soffice.exe"
    } else {
        "soffice"
    };
    if let Some(found) = find_in_path(executable) {
        return Some(found);
    }

    let mut candidates = Vec::new();

    if cfg!(windows) {
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = std::env::var_os(variable) {
                candidates.push(
                    PathBuf::from(base)
                        .join("LibreOffice")
                        .join("program")
                        .join("soffice.exe"),
                );
            }
        }
    } else if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from(
            "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        ));
    } else {
        candidates.extend([
            PathBuf::from("/usr/bin/soffice"),
            PathBuf::from("/usr/bin/libreoffice"),
            PathBuf::from("/snap/bin/libreoffice"),
        ]);
    }

    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn find_in_path(executable: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|directory| directory.join(executable))
        .find(|candidate| candidate.is_file())
}

fn file_url(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    format!("file:///{}", path.trim_start_matches('/'))
}

/// Runs a headless LibreOffice conversion and waits for the generated file.
///
/// `convert_to` is the value passed to `--convert-to` (optionally including
/// filter options), while `extension` is the extension LibreOffice will give
/// the produced file.
fn run_soffice(
    input: &Path,
    out_dir: &Path,
    soffice: Option<&Path>,
    timeout: Duration,
    convert_to: &str,
    extension: &str,
) -> Result<PathBuf, ConversionError> {
    if !input.is_file() {
        return Err(ConversionError::MissingInput(input.to_path_buf()));
    }

    let soffice = find_soffice(soffice).ok_or(ConversionError::LibreOfficeNotFound)?;
    std::fs::create_dir_all(out_dir)?;

    let deadline = Instant::now() + timeout;

    // An isolated profile keeps concurrent conversions from fighting over the
    // same LibreOffice user-profile lock.
    let profile = tempfile::Builder::new()
        .prefix("pdfword-lo-")
        .tempdir()
        .map_err(ConversionError::Io)?;
    let profile_arg = format!("-env:UserInstallation={}", file_url(profile.path()));

    let mut child = Command::new(&soffice)
        .arg("--headless")
        .arg("--nologo")
        .arg("--nofirststartwizard")
        .arg("--norestore")
        .arg(profile_arg)
        .arg("--convert-to")
        .arg(convert_to)
        .arg("--outdir")
        .arg(out_dir)
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            ConversionError::Process(format!("failed to start LibreOffice: {error}"))
        })?;

    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ConversionError::Timeout(timeout));
        }
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ConversionError::Io(error));
        }
    };

    if !status.success() {
        return Err(ConversionError::Process(format!(
            "LibreOffice exited with status {status}"
        )));
    }

    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| ConversionError::Unsupported(input.display().to_string()))?;
    let produced = out_dir.join(format!("{stem}.{extension}"));

    // On Windows soffice.exe can return before the detached conversion has
    // written its output, so poll for the result instead of checking once.
    loop {
        if produced.is_file() {
            return Ok(produced);
        }

        if Instant::now() >= deadline {
            return Err(ConversionError::Process(format!(
                "LibreOffice did not produce {}",
                produced.display()
            )));
        }

        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Converts a Word document to PDF with LibreOffice and returns the path of
/// the generated file (inside `out_dir`). Used fonts are embedded in the PDF.
pub fn docx_to_pdf(
    input: &Path,
    out_dir: &Path,
    soffice: Option<&Path>,
    timeout: Duration,
) -> Result<PathBuf, ConversionError> {
    run_soffice(input, out_dir, soffice, timeout, PDF_EXPORT_FILTER, "pdf")
}

/// Converts a WPS Writer document (`.wps`) or template (`.wpt`) to `.docx`
/// with LibreOffice's `libwps` import filter and returns the generated path.
pub fn wps_to_docx(
    input: &Path,
    out_dir: &Path,
    soffice: Option<&Path>,
    timeout: Duration,
) -> Result<PathBuf, ConversionError> {
    run_soffice(input, out_dir, soffice, timeout, "docx", "docx")
}

/// Converts a WPS document (`.wps` / `.wpt`, from WPS Office or Microsoft
/// Works) to PDF with LibreOffice and returns the generated path. Uses the
/// same font-embedding export filter as [`docx_to_pdf`].
pub fn wps_to_pdf(
    input: &Path,
    out_dir: &Path,
    soffice: Option<&Path>,
    timeout: Duration,
) -> Result<PathBuf, ConversionError> {
    run_soffice(input, out_dir, soffice, timeout, PDF_EXPORT_FILTER, "pdf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_file_urls_for_windows_and_unix_paths() {
        assert_eq!(file_url(Path::new("C:\\tmp\\x")), "file:///C:/tmp/x");
        assert_eq!(file_url(Path::new("/tmp/x")), "file:///tmp/x");
    }

    #[test]
    fn pdf_export_filter_requests_font_embedding() {
        assert!(PDF_EXPORT_FILTER.starts_with("pdf:writer_pdf_Export:"));
        assert!(PDF_EXPORT_FILTER.contains("EmbedStandardFonts"));
        assert!(PDF_EXPORT_FILTER.contains("EmbedOnlyUsedFonts"));
    }

    #[test]
    fn missing_input_is_reported() {
        let result = docx_to_pdf(
            Path::new("definitely-not-here.docx"),
            Path::new("."),
            None,
            Duration::from_secs(1),
        );
        assert!(matches!(result, Err(ConversionError::MissingInput(_))));
    }

    #[test]
    fn missing_wps_input_is_reported() {
        let result = wps_to_docx(
            Path::new("definitely-not-here.wps"),
            Path::new("."),
            None,
            Duration::from_secs(1),
        );
        assert!(matches!(result, Err(ConversionError::MissingInput(_))));
    }

    #[test]
    fn missing_wps_input_is_reported_for_pdf() {
        let result = wps_to_pdf(
            Path::new("definitely-not-here.wps"),
            Path::new("."),
            None,
            Duration::from_secs(1),
        );
        assert!(matches!(result, Err(ConversionError::MissingInput(_))));
    }
}
