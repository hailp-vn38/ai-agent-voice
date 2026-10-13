use super::*;

#[test]
fn docx_accepts_other_namespace_prefixes_and_rejects_unclosed_xml() {
    use std::io::Write;
    for (xml, valid) in [
        ("<x:document xmlns:x='http://schemas.openxmlformats.org/wordprocessingml/2006/main'><x:p><x:r><x:t>Hello</x:t></x:r></x:p></x:document>", true),
        ("<w:document><w:p><w:t>Hello</w:t>", false),
    ] {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        archive.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        archive.write_all(xml.as_bytes()).unwrap();
        let result = extract_docx(&archive.finish().unwrap().into_inner());
        if valid { assert_eq!(clean(&result.unwrap()), "Hello"); }
        else { assert!(result.is_err()); }
    }
}

#[test]
fn extracts_selectable_pdf_text() {
    use lopdf::{dictionary, Object};
    let mut document = lopdf::Document::with_version("1.5");
    let pages = document.new_object_id();
    let font = document.add_object(
        dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"},
    );
    let resources = document.add_object(dictionary! {"Font" => dictionary! {"F1" => font}});
    let content = document.add_object(lopdf::Stream::new(
        dictionary! {},
        b"BT /F1 12 Tf (Hello PDF) Tj ET".to_vec(),
    ));
    let page = document
        .add_object(dictionary! {"Type" => "Page", "Parent" => pages, "Contents" => content});
    document.objects.insert(
        pages,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1,
            "Resources" => resources, "MediaBox" => vec![0.into(),0.into(),600.into(),800.into()]
        }),
    );
    let catalog = document.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages});
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let (text, count) = extract_pdf(&bytes).unwrap();
    assert_eq!(clean(&text), "Hello PDF");
    assert_eq!(count, Some(1));
}
#[test]
fn headings_and_text() {
    let (text, titles) = extract_markdown("# Heading\n\nFirst **bold** paragraph.");
    assert_eq!(titles, vec!["Heading"]);
    assert!(clean(&text).contains("First bold paragraph"));
}
#[test]
fn extracts_docx_text() {
    use std::io::Write;
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    archive
        .start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(br#"<w:document><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:r><w:t>World</w:t></w:r></w:p></w:body></w:document>"#).unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    assert_eq!(clean(&extract_docx(&bytes).unwrap()), "Hello World");
}
#[test]
fn bad_pdf_fails() {
    assert!(extract_pdf(b"not-a-pdf").is_err());
}
#[test]
fn keyword_order_is_stable() {
    assert_eq!(keywords("rust rust document")[0]["term"], "rust");
}
