use std::path::Path;

use docx_rs::{
    AlignmentType, Docx, LineSpacing, PageMargin, PageOrientationType, Paragraph, Pic, Run,
};
use pdfium_render::prelude::*;

use crate::error::ConversionError;
use crate::pdf_to_docx::bind_pdfium;

/// Points per inch, used to convert a rendering scale factor into a DPI value.
const POINTS_PER_INCH: f32 = 72.0;
/// English Metric Units (EMU) per typographic point.
const EMU_PER_POINT: f32 = 12_700.0;
/// Twips (twentieths of a point) per typographic point.
const TWIPS_PER_POINT: f32 = 20.0;

/// Renders every PDF page to a PNG and embeds it in a Word document, so the
/// result looks like the source PDF but is not editable.
pub fn pdf_to_docx_exact(
    input: &Path,
    output: &Path,
    pdfium_dir: Option<&Path>,
    dpi: f32,
) -> Result<(), ConversionError> {
    if !input.is_file() {
        return Err(ConversionError::MissingInput(input.to_path_buf()));
    }

    let dpi = dpi.clamp(72.0, 600.0);
    let pdfium = bind_pdfium(pdfium_dir)?;
    let document = pdfium
        .load_pdf_from_file(input, None)
        .map_err(|error| ConversionError::Pdf(error.to_string()))?;

    let pages = document.pages();
    if pages.is_empty() {
        return Err(ConversionError::Pdf(
            "the PDF does not contain any pages".to_string(),
        ));
    }

    let first = pages
        .first()
        .map_err(|error| ConversionError::Pdf(error.to_string()))?;
    let page_width = first.width().value.max(1.0);
    let page_height = first.height().value.max(1.0);

    let mut docx = Docx::new()
        .page_size(twips(page_width), twips(page_height))
        .page_margin(
            PageMargin::new()
                .top(0)
                .left(0)
                .bottom(0)
                .right(0)
                .header(0)
                .footer(0)
                .gutter(0),
        );

    if page_width > page_height {
        docx = docx.page_orient(PageOrientationType::Landscape);
    }

    let scratch = tempfile::Builder::new()
        .prefix("pdfword-exact-")
        .tempdir()?;
    let scale = dpi / POINTS_PER_INCH;

    for (index, page) in pages.iter().enumerate() {
        let width = page.width().value.max(1.0);
        let height = page.height().value.max(1.0);

        let config =
            PdfRenderConfig::new().set_target_size(pixels(width, scale), pixels(height, scale));
        let bitmap = page
            .render_with_config(&config)
            .map_err(|error| ConversionError::Render(error.to_string()))?;
        let image = bitmap
            .as_image()
            .map_err(|error| ConversionError::Render(error.to_string()))?;

        let png_path = scratch.path().join(format!("page-{index}.png"));
        image
            .save(&png_path)
            .map_err(|error| ConversionError::Render(error.to_string()))?;
        let bytes = std::fs::read(&png_path)?;

        // Shrink the image very slightly so Word never pushes it onto an extra
        // page because of line metrics and rounding.
        let fit = (page_width / width).min(page_height / height).min(1.0);
        let display_width = width * fit * 0.998;
        let display_height = height * fit * 0.998;

        let picture = Pic::new(&bytes).size(emu(display_width), emu(display_height));
        let paragraph = Paragraph::new()
            .align(AlignmentType::Center)
            .page_break_before(index > 0)
            .line_spacing(LineSpacing::new().before(0).after(0).line(240))
            .add_run(Run::new().size(2).add_image(picture));
        docx = docx.add_paragraph(paragraph);
    }

    let file = std::fs::File::create(output)?;
    docx.build()
        .pack(file)
        .map_err(|error| ConversionError::Docx(error.to_string()))?;

    Ok(())
}

fn twips(points: f32) -> u32 {
    (points * TWIPS_PER_POINT).round().max(1.0) as u32
}

fn emu(points: f32) -> u32 {
    (points * EMU_PER_POINT).round().max(1.0) as u32
}

fn pixels(points: f32, scale: f32) -> i32 {
    (points * scale).round().max(1.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_points_to_twips_and_emu() {
        assert_eq!(twips(72.0), 1440);
        assert_eq!(emu(72.0), 914_400);
    }

    #[test]
    fn computes_pixel_sizes_at_the_requested_scale() {
        assert_eq!(pixels(72.0, 2.0), 144);
        assert_eq!(pixels(0.1, 1.0), 1);
    }

    #[test]
    fn missing_input_is_reported() {
        let result = pdf_to_docx_exact(
            Path::new("definitely-not-here.pdf"),
            Path::new("out.docx"),
            None,
            200.0,
        );
        assert!(matches!(result, Err(ConversionError::MissingInput(_))));
    }
}
